use super::*;
use crate::core::config::Language;

#[test]
fn ffi_error_code_variant_names_include_the_canonical_error_type() {
    assert_eq!(
        ffi_error_code_variant_name("sample::RequestError", "Unavailable"),
        "SampleRequestErrorUnavailable"
    );
    assert_ne!(
        ffi_error_code_variant_name("sample::RequestError", "Unavailable"),
        ffi_error_code_variant_name("sample::StorageError", "Unavailable")
    );
}

#[test]
fn ffi_error_code_variant_names_do_not_repeat_error_at_the_boundary() {
    assert_eq!(
        ffi_error_code_variant_name("sample::ParserError", "ErrorLanguageNotFound"),
        "SampleParserErrorLanguageNotFound"
    );
}

#[test]
fn ffi_error_code_variant_names_collapse_repeats_inside_the_type_path() {
    let cases = [
        // A module named after its error type is the common stutter source.
        (("sample::error::Error", "NotFound"), "SampleErrorNotFound"),
        (("sample_lib::error::Error", "NotFound"), "SampleLibErrorNotFound"),
        // Case differences alone do not make two consecutive words distinct.
        (("sample::Error::Error", "NotFound"), "SampleErrorNotFound"),
        // A repeat that is not adjacent carries meaning and is preserved.
        (("sample::error::Error", "QueryError"), "SampleErrorQueryError"),
        (("sample::error::codec::Error", "Decode"), "SampleErrorCodecErrorDecode"),
        // Paths without a repeat are untouched.
        (("sample::RequestError", "Unavailable"), "SampleRequestErrorUnavailable"),
    ];
    for ((error_type, variant), expected) in cases {
        assert_eq!(
            ffi_error_code_variant_name(error_type, variant),
            expected,
            "{error_type}::{variant}"
        );
    }
}

#[test]
fn ffi_error_code_variant_names_stay_distinct_across_sibling_error_types() {
    // Path collapsing must not erase the type that distinguishes two taxonomies.
    assert_ne!(
        ffi_error_code_variant_name("sample::error::Error", "NotFound"),
        ffi_error_code_variant_name("sample::storage::Error", "NotFound")
    );
}

#[test]
fn eliding_error_at_the_boundary_can_alias_two_variants_of_one_type() {
    // Documented limitation, unchanged from the previous implementation: a type
    // carrying both `ErrorFoo` and `Foo` folds onto a single name. C rejects the
    // duplicate enumerator at compile time, so this surfaces loudly rather than
    // silently mapping a code incorrectly. Deliberately not "fixed" by dropping the elision,
    // which would reintroduce the `ErrorError` stutter this pass removes. ~keep
    assert_eq!(
        ffi_error_code_variant_name("sample::error::Error", "ErrorFoo"),
        ffi_error_code_variant_name("sample::error::Error", "Foo")
    );
}

#[test]
fn ffi_builtin_error_code_prefix_namespaces_members_per_project() {
    assert_eq!(ffi_builtin_error_code_prefix("sample"), "SampleAlef");
    assert_eq!(ffi_builtin_error_code_prefix("ts_pack"), "TsPackAlef");
    assert_ne!(
        ffi_builtin_error_code_prefix("sample"),
        ffi_builtin_error_code_prefix("other")
    );
}

#[test]
fn serde_wire_name_applies_rename_all_strategies() {
    let cases = [
        ("HttpStatus", Some("lowercase"), "httpstatus"),
        ("HttpStatus", Some("UPPERCASE"), "HTTPSTATUS"),
        ("http_status", Some("PascalCase"), "HttpStatus"),
        ("http_status", Some("camelCase"), "httpStatus"),
        ("HttpStatus", Some("snake_case"), "http_status"),
        ("HttpStatus", Some("SCREAMING_SNAKE_CASE"), "HTTP_STATUS"),
        ("HttpStatus", Some("kebab-case"), "http-status"),
        ("HttpStatus", Some("SCREAMING-KEBAB-CASE"), "HTTP-STATUS"),
        ("HttpStatus", None, "HttpStatus"),
        ("HttpStatus", Some("unknown"), "HttpStatus"),
        ("Rdfa", Some("snake_case"), "rdfa"),
        ("HTMLParser", Some("snake_case"), "html_parser"),
    ];

    for (name, rename_all, expected) in cases {
        assert_eq!(
            apply_serde_rename_all(name, rename_all),
            expected,
            "rename_all={rename_all:?} name={name}"
        );
    }
}

#[test]
fn serde_rename_wins_over_rename_all() {
    assert_eq!(
        serde_wire_name("content_type", Some("Content-Type"), Some("camelCase")),
        "Content-Type"
    );
    assert_eq!(
        wire_variant_value("HttpStatus", Some("http-status"), Some("snake_case")),
        "http-status"
    );
    assert_eq!(
        wire_variant_value("HttpStatus", None, Some("snake_case")),
        "http_status"
    );
}

// `EnumDef::rename_all_fields` cases the FIELD names of a struct-shaped variant's payload; it is
// a distinct serde namespace from `EnumDef::serde_rename_all`, which cases VARIANT names. A call
// site computing a struct-variant field's wire name must resolve it through `wire_field_name`
// with the field's own `serde_rename` and the enum's `rename_all_fields` -- never the enum's
// `serde_rename_all`, which is the exact confusion this precedence chain guards against. ~keep
#[test]
fn struct_variant_field_wire_name_prefers_field_rename_over_enum_rename_all_fields() {
    let field = crate::core::ir::FieldDef {
        name: "inner_radius".to_string(),
        serde_rename: Some("radius".to_string()),
        ..crate::core::ir::FieldDef::default()
    };
    let enum_def = crate::core::ir::EnumDef {
        rename_all_fields: Some("camelCase".to_string()),
        ..crate::core::ir::EnumDef::default()
    };

    assert_eq!(
        wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            enum_def.rename_all_fields.as_deref()
        ),
        "radius",
        "an explicit field-level serde_rename must win over the enum's rename_all_fields"
    );
}

#[test]
fn struct_variant_field_wire_name_falls_back_to_rename_all_fields_without_field_rename() {
    let field = crate::core::ir::FieldDef {
        name: "inner_radius".to_string(),
        ..crate::core::ir::FieldDef::default()
    };
    let enum_def = crate::core::ir::EnumDef {
        rename_all_fields: Some("camelCase".to_string()),
        ..crate::core::ir::EnumDef::default()
    };

    assert_eq!(
        wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            enum_def.rename_all_fields.as_deref()
        ),
        "innerRadius",
        "with no field-level rename, the enum's rename_all_fields must case the raw field name"
    );
}

#[test]
fn struct_variant_field_wire_name_falls_back_to_raw_name_with_no_rule_at_all() {
    let field = crate::core::ir::FieldDef {
        name: "inner_radius".to_string(),
        ..crate::core::ir::FieldDef::default()
    };
    let enum_def = crate::core::ir::EnumDef::default();

    assert_eq!(
        wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            enum_def.rename_all_fields.as_deref()
        ),
        "inner_radius"
    );
}

/// `rename_all` (variant names) and `rename_all_fields` (struct-variant field names) are
/// independent serde container attributes: an enum may set either, both, or neither, and one
/// must never influence the other's resolved value. ~keep
#[test]
fn rename_all_and_rename_all_fields_are_independent() {
    let variant_name_only = crate::core::ir::EnumDef {
        serde_rename_all: Some("SCREAMING_SNAKE_CASE".to_string()),
        ..crate::core::ir::EnumDef::default()
    };
    assert_eq!(variant_name_only.rename_all_fields, None);

    let field_name_only = crate::core::ir::EnumDef {
        rename_all_fields: Some("kebab-case".to_string()),
        ..crate::core::ir::EnumDef::default()
    };
    assert_eq!(field_name_only.serde_rename_all, None);

    let both = crate::core::ir::EnumDef {
        serde_rename_all: Some("SCREAMING_SNAKE_CASE".to_string()),
        rename_all_fields: Some("camelCase".to_string()),
        ..crate::core::ir::EnumDef::default()
    };
    assert_eq!(
        wire_variant_value("InnerRadius", None, both.serde_rename_all.as_deref()),
        "INNER_RADIUS",
        "serde_rename_all still governs variant-name casing when both are set"
    );
    assert_eq!(
        wire_field_name("inner_radius", None, both.rename_all_fields.as_deref()),
        "innerRadius",
        "rename_all_fields still governs field-name casing when both are set"
    );
}

#[test]
fn public_identifiers_are_separate_from_wire_names() {
    assert_eq!(
        public_field_name(Language::Node, "content_type", Some("contentTypeOverride")),
        "contentTypeOverride"
    );
    assert_eq!(
        wire_field_name("content_type", Some("Content-Type"), Some("camelCase")),
        "Content-Type"
    );
}

#[test]
fn public_host_identifier_applies_language_casing_and_keywords() {
    let cases = [
        (Language::Python, PublicIdentifierKind::Field, "class", "class_"),
        (
            Language::Node,
            PublicIdentifierKind::Function,
            "request_url",
            "requestUrl",
        ),
        (
            Language::Go,
            PublicIdentifierKind::Function,
            "request_url",
            "RequestURL",
        ),
        (
            Language::Go,
            PublicIdentifierKind::Parameter,
            "request_url",
            "requestURL",
        ),
        (
            Language::Csharp,
            PublicIdentifierKind::Method,
            "graphql_route",
            "GraphQLRoute",
        ),
        (
            Language::Ruby,
            PublicIdentifierKind::EnumVariant,
            "HTTPStatus",
            "http_status",
        ),
    ];

    for (lang, kind, name, expected) in cases {
        assert_eq!(public_host_identifier(lang, kind, name), expected);
    }
}

#[test]
fn identifier_context_controls_escaping() {
    assert_eq!(
        escape_identifier_for(Language::Swift, "protocol", IdentifierContext::SwiftSource),
        "`protocol`"
    );
    assert_eq!(
        escape_identifier_for(Language::Swift, "protocol", IdentifierContext::SwiftRustShim),
        "protocol_"
    );
    assert_eq!(
        escape_identifier_for(Language::Kotlin, "object", IdentifierContext::KotlinSource),
        "`object`"
    );
    assert_eq!(
        escape_identifier_for(Language::Kotlin, "object", IdentifierContext::KotlinRustBridge),
        "object_"
    );
    assert_eq!(
        escape_identifier_for(Language::Rust, "type", IdentifierContext::InternalRust),
        "r#type"
    );
}

#[test]
fn dart_identifier_context_handles_core_types_and_tuple_fields() {
    assert_eq!(dart_type_identifier("List", Some("NodeContent")), "NodeContentList");
    assert_eq!(dart_type_identifier("List", None), "ListNode");
    assert_eq!(dart_value_identifier("required"), "required_");
    assert_eq!(dart_tuple_field_identifier("0"), "field0");
}

#[test]
fn abi_symbol_components_are_sanitized_and_joined() {
    assert_eq!(
        abi_symbol_from_components(["my-lib", "HTTPStatus", "Content-Type", "2xx"]),
        "my_lib_httpstatus_content_type_2xx"
    );
    assert_eq!(abi_symbol("ffi", "HTTPStatus"), "ffi_http_status");
}

#[test]
fn identifier_validation_is_contextual() {
    assert!(validate_identifier(Language::Swift, "`protocol`", IdentifierContext::SwiftSource).is_ok());
    assert!(validate_identifier(Language::Rust, "r#type", IdentifierContext::InternalRust).is_ok());
    assert!(validate_identifier(Language::Node, "requestUrl", IdentifierContext::PublicMember).is_ok());
    assert!(validate_identifier(Language::Node, "Content-Type", IdentifierContext::PublicMember).is_err());
    assert!(validate_identifier(Language::Node, "Content-Type", IdentifierContext::Wire).is_ok());
}

#[test]
fn name_collision_detection_groups_distinct_originals() {
    let collisions = detect_name_collisions(["foo_bar", "fooBar", "baz"], |name| {
        public_host_identifier(Language::Node, PublicIdentifierKind::Field, name)
    });

    assert_eq!(
        collisions,
        vec![NameCollision {
            generated: "fooBar".to_string(),
            originals: vec!["foo_bar".to_string(), "fooBar".to_string()],
        }]
    );
}

#[test]
fn test_to_go_name_html_initialism() {
    assert_eq!(to_go_name("html"), "HTML");
}

#[test]
fn test_to_go_name_url_initialism() {
    assert_eq!(to_go_name("url"), "URL");
}

#[test]
fn test_to_go_name_id_initialism() {
    assert_eq!(to_go_name("id"), "ID");
}

#[test]
fn test_to_go_name_plain_word() {
    assert_eq!(to_go_name("links"), "Links");
}

#[test]
fn test_to_go_name_user_id() {
    assert_eq!(to_go_name("user_id"), "UserID");
}

#[test]
fn test_to_go_name_request_url() {
    assert_eq!(to_go_name("request_url"), "RequestURL");
}

#[test]
fn test_to_go_name_http_status() {
    assert_eq!(to_go_name("http_status"), "HTTPStatus");
}

#[test]
fn test_to_go_name_json_body() {
    assert_eq!(to_go_name("json_body"), "JSONBody");
}

#[test]
fn test_go_param_name_base_url() {
    assert_eq!(go_param_name("base_url"), "baseURL");
}

#[test]
fn test_go_param_name_user_id() {
    assert_eq!(go_param_name("user_id"), "userID");
}

#[test]
fn test_go_param_name_api_key() {
    assert_eq!(go_param_name("api_key"), "apiKey");
}

#[test]
fn test_go_param_name_plain() {
    assert_eq!(go_param_name("json"), "json");
}

#[test]
fn go_package_name_from_module_takes_the_last_segment() {
    let cases = [
        ("github.com/org/my-lib", "mylib"),
        ("binding", "binding"),
        ("example.invalid/samplecrate", "samplecrate"),
        ("", "binding"),
    ];
    for (module_path, expected) in cases {
        assert_eq!(
            go_package_name_from_module(module_path),
            expected,
            "module_path={module_path:?}"
        );
    }
}

// Table-driven regression for the bug where the e2e/docs Go snippet generator referenced the
// raw Rust-side error type name while the Go backend's own error generator
// (`gen_go_error_struct`, which calls this exact function) strips the package prefix -- a
// leading case-insensitive match of the Go package name is removed from the Rust error type
// name to avoid revive's stutter lint. Any caller that re-derives this rule instead of calling
// `go_error_type_name` can drift from what the Go backend actually declares; this test pins the
// shared rule itself. ~keep
#[test]
fn go_error_type_name_strips_a_matching_package_prefix() {
    let cases = [
        // Real-world shape: crate `error_type = "SampleCrateError"`, Go package `samplecrate`.
        ("SampleCrateError", "samplecrate", "Error"),
        ("SampleLlmError", "samplellm", "Error"),
        // No overlap between the error type and the package name: left untouched.
        ("ConversionError", "converter", "ConversionError"),
        // Negative control: an unrelated error name sharing no prefix with the package.
        ("ParseError", "samplecrate", "ParseError"),
        // Exact match (type name equals package name with no suffix) is not stripped -- the
        // `len() > pkg_lower.len()` guard exists so a package literally named after its own
        // error type is left with a non-empty identifier.
        ("Error", "error", "Error"),
        // Empty package name: nothing to strip.
        ("Error", "", "Error"),
    ];
    for (error_name, pkg_name, expected) in cases {
        assert_eq!(
            go_error_type_name(error_name, pkg_name),
            expected,
            "error_name={error_name:?} pkg_name={pkg_name:?}"
        );
    }
}

#[test]
fn pascal_to_snake_normal_case() {
    assert_eq!(pascal_to_snake("MyType"), "my_type");
}

#[test]
fn pascal_to_snake_rdfa() {
    assert_eq!(pascal_to_snake("Rdfa"), "rdfa");
}

/// The discriminating case for `pascal_to_snake`'s trailing-word-length heuristic: `RDF` is a 3-letter
/// uppercase run followed by a single lowercase `a`, which must stay part of the same word
/// (`rdfa`) rather than split into `rd_fa` the way a real two-letter trailing word would
/// (`IOError` -> `io_error`, covered below). Before this threshold existed,
/// `docs::naming::enum_variant_name` carried a hardcoded `if name == "RDFa"` table specifically
/// because this function produced `rd_fa` for the real Rust variant name. ~keep
#[test]
fn pascal_to_snake_rdfa_all_caps_acronym_with_short_suffix() {
    assert_eq!(pascal_to_snake("RDFa"), "rdfa");
    assert_eq!(pascal_to_screaming_snake("RDFa"), "RDFA");
}

#[test]
fn pascal_to_snake_html_parser() {
    assert_eq!(pascal_to_snake("HTMLParser"), "html_parser");
}

#[test]
fn pascal_to_snake_xml_http_request() {
    assert_eq!(pascal_to_snake("XMLHttpRequest"), "xml_http_request");
}

#[test]
fn pascal_to_snake_io_error() {
    assert_eq!(pascal_to_snake("IOError"), "io_error");
}

#[test]
fn pascal_to_snake_url_path() {
    assert_eq!(pascal_to_snake("URLPath"), "url_path");
}

#[test]
fn pascal_to_snake_jsonld_all_caps() {
    assert_eq!(pascal_to_snake("JSONLD"), "jsonld");
}

#[test]
fn pascal_to_snake_camel_case() {
    assert_eq!(pascal_to_snake("myField"), "my_field");
}

#[test]
fn pascal_to_snake_already_snake() {
    assert_eq!(pascal_to_snake("already_snake"), "already_snake");
}

#[test]
fn pascal_to_snake_empty() {
    assert_eq!(pascal_to_snake(""), "");
}

#[test]
fn pascal_to_screaming_snake_rdfa() {
    assert_eq!(pascal_to_screaming_snake("Rdfa"), "RDFA");
}

#[test]
fn pascal_to_screaming_snake_html_parser() {
    assert_eq!(pascal_to_screaming_snake("HTMLParser"), "HTML_PARSER");
}

#[test]
fn pascal_to_screaming_snake_my_type() {
    assert_eq!(pascal_to_screaming_snake("MyType"), "MY_TYPE");
}

#[test]
fn pascal_to_pascal_rdfa() {
    assert_eq!(pascal_to_pascal("RDFa"), "Rdfa");
}

#[test]
fn pascal_to_pascal_io_error() {
    assert_eq!(pascal_to_pascal("IOError"), "IoError");
}

#[test]
fn pascal_to_pascal_html_parser() {
    assert_eq!(pascal_to_pascal("HTMLParser"), "HtmlParser");
}

#[test]
fn pascal_to_pascal_xml_http_request() {
    assert_eq!(pascal_to_pascal("XMLHttpRequest"), "XmlHttpRequest");
}

#[test]
fn pascal_to_pascal_jsonld_all_caps() {
    assert_eq!(pascal_to_pascal("JSONLD"), "Jsonld");
}

#[test]
fn pascal_to_pascal_my_type() {
    assert_eq!(pascal_to_pascal("MyType"), "MyType");
}

#[test]
fn pascal_to_pascal_camel_case_input() {
    assert_eq!(pascal_to_pascal("myField"), "MyField");
}

#[test]
fn pascal_to_pascal_empty() {
    assert_eq!(pascal_to_pascal(""), "");
}

#[test]
fn pascal_to_camel_rdfa() {
    assert_eq!(pascal_to_camel("RDFa"), "rdfa");
}

#[test]
fn pascal_to_camel_io_error() {
    assert_eq!(pascal_to_camel("IOError"), "ioError");
}

#[test]
fn pascal_to_camel_html_parser() {
    assert_eq!(pascal_to_camel("HTMLParser"), "htmlParser");
}

#[test]
fn pascal_to_camel_xml_http_request() {
    assert_eq!(pascal_to_camel("XMLHttpRequest"), "xmlHttpRequest");
}

#[test]
fn pascal_to_camel_jsonld_all_caps() {
    assert_eq!(pascal_to_camel("JSONLD"), "jsonld");
}

#[test]
fn pascal_to_camel_my_type() {
    assert_eq!(pascal_to_camel("MyType"), "myType");
}

#[test]
fn pascal_to_camel_already_camel() {
    assert_eq!(pascal_to_camel("myField"), "myField");
}

#[test]
fn pascal_to_camel_empty() {
    assert_eq!(pascal_to_camel(""), "");
}

#[test]
fn test_to_csharp_name_graphql_route_config() {
    assert_eq!(to_csharp_name("graphql_route_config"), "GraphQLRouteConfig");
}

#[test]
fn test_to_csharp_name_http_status_no_acronym() {
    assert_eq!(to_csharp_name("http_status"), "HttpStatus");
}

#[test]
fn test_to_csharp_name_to_json_no_acronym() {
    assert_eq!(to_csharp_name("to_json"), "ToJson");
}

#[test]
fn test_to_csharp_name_plain() {
    assert_eq!(to_csharp_name("my_field"), "MyField");
}

#[test]
fn test_csharp_type_name_heck_corrupted() {
    assert_eq!(csharp_type_name("GraphQlRouteConfig"), "GraphQLRouteConfig");
}

#[test]
fn test_csharp_type_name_already_correct() {
    assert_eq!(csharp_type_name("GraphQLRouteConfig"), "GraphQLRouteConfig");
}

#[test]
fn test_csharp_type_name_http_status_no_acronym() {
    assert_eq!(csharp_type_name("HttpStatus"), "HttpStatus");
}

#[test]
fn test_csharp_type_name_three_letter_acronyms() {
    assert_eq!(csharp_type_name("Uri"), "Uri");
    assert_eq!(csharp_type_name("URI"), "Uri");
    assert_eq!(csharp_type_name("Xml"), "Xml");
    assert_eq!(csharp_type_name("XML"), "Xml");
    assert_eq!(csharp_type_name("Json"), "Json");
    assert_eq!(csharp_type_name("JSON"), "Json");
}

/// Issue #449: `public_type_name`'s Go and C# arms used to run `heck::to_pascal_case()` on the
/// name *before* handing it to `go_type_name`/`csharp_type_name`. Every real Go/C# backend call
/// site (`go_type_name(&typ.name)`, `csharp_type_name(&typ.name)`) passes the raw IR type name
/// with no such pre-step, and `heck::to_pascal_case()` re-segments an irregular acronym run that
/// is not in either language's initialism list (`RDFaChunk` -> `RdFaChunk`), so
/// `public_host_identifier` (what `src/docs` and `crate::e2e::validate_call_class` use) silently
/// disagreed with what the backend actually emits.
///
/// Table-driven over the acronym names measured for issue #449: for every one of them, the
/// public host identifier for `PublicIdentifierKind::Type` must now be byte-identical to calling
/// the backend's own naming primitive directly, for both Go and C#.
#[test]
fn go_and_csharp_public_type_name_matches_the_backend_naming_primitive_for_acronym_runs() {
    let names = [
        "IOError",
        "RDFaChunk",
        "JSONLD",
        "SSRFPolicy",
        "DBHandle",
        "SQLiteDB",
        "GRPCClient",
        // Already round-trip correctly today because they ARE in Go's INITIALISMS list; kept
        // here so a future change to the pre-step can't quietly break the cases that used to work.
        "HTMLParser",
        "URLPath",
        "APIKey",
        "HTTPResponse",
    ];

    for name in names {
        assert_eq!(
            public_host_identifier(Language::Go, PublicIdentifierKind::Type, name),
            go_type_name(name),
            "Go public type name must match go_type_name({name:?}) exactly"
        );
        assert_eq!(
            public_host_identifier(Language::Csharp, PublicIdentifierKind::Type, name),
            csharp_type_name(name),
            "C# public type name must match csharp_type_name({name:?}) exactly"
        );
    }
}

/// Pins the exact pre-fix-vs-post-fix spelling for the case the issue was filed over, so a
/// regression back to the `to_pascal_case()` pre-step fails on a concrete value, not just an
/// equality-with-itself comparison.
#[test]
fn go_public_type_name_no_longer_mis_segments_an_irregular_acronym_run() {
    assert_eq!(
        public_host_identifier(Language::Go, PublicIdentifierKind::Type, "RDFaChunk"),
        "RDFaChunk",
        "must match what the Go backend actually emits, not heck's re-segmented RdFaChunk"
    );
}

/// `kotlin_android_wrapper_object_name` passes a non-PascalCase (crate) name through this
/// module. Issue #455 made `public_type_name`'s `KotlinAndroid` arm an identity function (it now
/// assumes an already-PascalCase IR type name), so this helper was moved onto
/// [`cased_pascal_type_name`] -- the "case an arbitrary name" entry point -- instead. Its
/// observable output must not change.
#[test]
fn kotlin_android_wrapper_object_name_still_pascal_cases_a_kebab_case_crate_name() {
    assert_eq!(kotlin_android_wrapper_object_name("sample-parser-rs"), "SampleParser");
    assert_eq!(kotlin_android_wrapper_object_name("document_tools"), "DocumentTools");
}

/// Issue #455: thirteen backends (pyo3, napi, magnus, php, rustler, java, kotlin,
/// kotlin_android, swift, dart, gleam, zig, wasm) declare their public type from an IR type name
/// (`typ.name`/`enum_def.name`) VERBATIM -- none of them run it through
/// `heck::to_pascal_case()` or any acronym pass first. `public_type_name`'s catch-all arm used to
/// run every one of these languages through `heck::to_pascal_case()` regardless, which
/// re-segments an acronym run that is not in any per-language initialism list (`RDFaChunk` ->
/// `RdFaChunk`), so `public_host_identifier` -- what `src/docs` and
/// `crate::e2e::validate_call_class` use -- silently disagreed with what every one of these
/// backends actually emits. Table-driven over the acronym names measured for the issue, over
/// every affected `Language` variant, so a regression in any single arm fails immediately rather
/// than only being caught for whichever language a hand-picked example happened to use.
#[test]
fn verbatim_backends_public_type_name_no_longer_re_cases_an_already_pascal_case_name() {
    let names = [
        "IOError",
        "HTMLParser",
        "JSONLD",
        "SSRFPolicy",
        "DBHandle",
        "SQLiteDB",
        "GRPCClient",
        "URLPath",
        "RDFaChunk",
        // The two of the eleven measured names that happened to survive heck's PascalCase
        // round-trip unchanged; kept so a future regression can't hide behind "only the
        // already-broken cases were checked".
        "DefaultClient",
        "IPv4Addr",
    ];
    let langs = [
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Elixir,
        Language::Java,
        Language::Kotlin,
        Language::KotlinAndroid,
        Language::Swift,
        Language::Dart,
        Language::Gleam,
        Language::Zig,
        Language::Wasm,
        Language::R,
    ];
    for lang in langs {
        for name in names {
            assert_eq!(
                public_host_identifier(lang, PublicIdentifierKind::Type, name),
                name,
                "{lang:?} must leave an already-PascalCase IR type name {name:?} untouched, \
                 matching what its backend declares verbatim"
            );
        }
    }
}

/// Issue #467: `zig_error_variant_component` was moved off `public_host_identifier(Zig, Type,
/// ...)` and onto `cased_public_type_name` so a genuinely uncased IR name (see
/// `src/backends/zig/gen_bindings/errors.rs`) gets PascalCased again. That move must not disturb
/// `public_host_identifier(Zig, Type, ...)` itself -- the path Zig's actual `pub const {typ.name}
/// = struct` type declarations still go through -- which must keep leaving an irregular acronym
/// run alone exactly as #455 fixed it. Pinned on Zig specifically (the backend #467 regressed),
/// with the exact acronym example from the issue report.
#[test]
fn zig_public_type_name_still_leaves_an_irregular_acronym_run_alone_after_467() {
    assert_eq!(
        public_host_identifier(Language::Zig, PublicIdentifierKind::Type, "HTTPSProxyConfig"),
        "HTTPSProxyConfig",
        "Zig's `pub const {{typ.name}} = struct` declares the IR type name verbatim; a \
         to_pascal_case() pre-step here would re-segment the acronym run, regressing #455"
    );
}

/// Pins the exact pre-fix-vs-post-fix spelling for the case the issue was filed over, on two of
/// the affected languages, so a regression back to the `to_pascal_case()` pre-step fails on a
/// concrete wrong value (`RdFaChunk`), not just an equality-with-itself comparison.
#[test]
fn kotlin_and_python_public_type_name_no_longer_mis_segment_an_irregular_acronym_run() {
    assert_eq!(
        public_host_identifier(Language::Kotlin, PublicIdentifierKind::Type, "RDFaChunk"),
        "RDFaChunk",
        "must match what the kotlin backend's `data class` declaration actually emits, not \
         heck's re-segmented RdFaChunk"
    );
    assert_eq!(
        public_host_identifier(Language::Python, PublicIdentifierKind::Type, "RDFaChunk"),
        "RDFaChunk",
        "must match what pyo3's #[pyclass] actually emits (no name override), not heck's \
         re-segmented RdFaChunk"
    );
}

/// The input-contract split issue #455 introduced: [`cased_public_type_name`] is the entry point
/// for a name that genuinely needs case conversion (a crate name, or a snake_case adapter/method
/// name reused as a type-name fragment), as opposed to `public_host_identifier(_,
/// PublicIdentifierKind::Type, _)` which now assumes its input is already PascalCase and leaves
/// it alone. A kebab-case crate name must still come out fully PascalCased through this path, for
/// every language that has a real crate-name-to-class-name caller today (Kotlin via
/// `kotlin_pascal_case`/`crate_facade_class_names`, KotlinAndroid via
/// `kotlin_android_wrapper_object_name`'s sibling `cased_pascal_type_name`) and for Go/C# per
/// #449's pre-existing crate-name contract.
#[test]
fn cased_public_type_name_still_fully_pascal_cases_an_uncased_name() {
    let cases = [
        (Language::Kotlin, "sample-widget-rs", "SampleWidgetRs"),
        (Language::Kotlin, "crawl_stream", "CrawlStream"),
        (Language::KotlinAndroid, "sample-widget-rs", "SampleWidgetRs"),
        (Language::Go, "sample_widget", "SampleWidget"),
        (Language::Csharp, "sample_widget", "SampleWidget"),
    ];
    for (lang, input, expected) in cases {
        assert_eq!(
            cased_public_type_name(lang, input),
            expected,
            "{lang:?} must fully case {input:?} into PascalCase, not leave it untouched"
        );
    }
}

/// `node_type_name` must be the identity function: the NAPI-RS `.d.ts` emitter never applies
/// the Rust-side `Js` wrapper prefix to a TypeScript type name, on the declaration side or the
/// reference side. Table-driven so a future accidental prefix-adding edit fails immediately.
#[test]
fn node_type_name_never_adds_the_js_wrapper_prefix() {
    let cases = [
        ("Message", "Message"),
        ("ChatCompletionRequest", "ChatCompletionRequest"),
        ("StopSequence", "StopSequence"),
        ("Js", "Js"),
    ];
    for (input, expected) in cases {
        assert_eq!(node_type_name(input), expected, "node_type_name({input:?})");
    }
}

/// `qualified_type_path` is the single decision point for "does this name already carry a
/// package". Table-driven: every emitter that prefixes a JVM/.NET package onto a configured type
/// name routes through here, so a regression in any row doubles a package prefix in generated code.
#[test]
fn qualified_type_path_prefixes_bare_names_and_leaves_qualified_ones_alone() {
    let cases = [
        (
            "dev.sample.bindings",
            "SampleClient",
            "dev.sample.bindings.SampleClient",
        ),
        (
            "dev.sample.bindings",
            "dev.sample.bindings.SampleClient",
            "dev.sample.bindings.SampleClient",
        ),
        (
            "dev.sample.bindings",
            "other.pkg.SampleClient",
            "other.pkg.SampleClient",
        ),
        ("dev.sample.bindings", "*", "dev.sample.bindings.*"),
        ("", "SampleClient", "SampleClient"),
    ];
    for (package, type_name, expected) in cases {
        assert_eq!(
            qualified_type_path(package, type_name),
            expected,
            "qualified_type_path({package:?}, {type_name:?})"
        );
    }
}

/// Moved here verbatim from an inline `mod duration_wire_tests` in `naming.rs` when
/// `naming::ts_property_key` was added: `naming.rs` sat at exactly the 1,000-line cap
/// `tests/file_size_ratchet.rs` enforces, so the `mod` declaration had to be paid for. Uses
/// absolute paths rather than `use super::*` so it does not depend on a glob-of-a-glob resolving.
mod duration_wire_tests {
    use crate::codegen::naming::field_uses_duration_map_wire;
    use crate::core::ir::{FieldDef, PrimitiveType, TypeRef};

    fn duration_field(serde_with: Option<&str>) -> FieldDef {
        FieldDef {
            ty: TypeRef::Duration,
            serde_with: serde_with.map(str::to_string),
            ..FieldDef::default()
        }
    }

    #[test]
    fn duration_field_without_serde_with_uses_the_derived_map_wire() {
        assert!(field_uses_duration_map_wire(&duration_field(None)));
    }

    #[test]
    fn duration_field_with_serde_with_uses_the_scalar_wire() {
        assert!(!field_uses_duration_map_wire(&duration_field(Some("duration_ms"))));
    }

    #[test]
    fn non_duration_field_never_uses_the_duration_map_wire() {
        let mut field = duration_field(None);
        field.ty = TypeRef::Primitive(PrimitiveType::U64);
        assert!(!field_uses_duration_map_wire(&field));
    }
}

#[test]
fn underscore_camel_case_capitalizes_only_after_an_underscore() {
    let cases = [
        ("hello_world", "helloWorld"),
        ("no_underscores_here", "noUnderscoresHere"),
        ("already", "already"),
        ("", ""),
        ("a_b_c", "aBC"),
        ("trailing_", "trailing"),
        ("double__underscore", "doubleUnderscore"),
    ];
    for (input, expected) in cases {
        assert_eq!(underscore_camel_case(input), expected, "input: {input}");
    }
}

#[test]
fn underscore_camel_case_preserves_case_that_heck_would_re_split() {
    // These are the cases the helper exists for. `to_node_name` (heck) re-splits an existing
    // case run and drops a leading underscore; a caller matching a name another generator
    // already emitted must not. If these two ever agree, the separate helper is pointless.
    let divergent = [
        ("my_URL", "myURL", "myUrl"),
        ("_raw", "Raw", "raw"),
        ("parse_HTML", "parseHTML", "parseHtml"),
    ];
    for (input, preserving, heck) in divergent {
        assert_eq!(underscore_camel_case(input), preserving, "input: {input}");
        assert_eq!(to_node_name(input), heck, "input: {input}");
        assert_ne!(underscore_camel_case(input), to_node_name(input), "input: {input}");
    }
}
