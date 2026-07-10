use super::*;
use crate::core::config::Language;

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
fn pascal_to_snake_normal_case() {
    assert_eq!(pascal_to_snake("MyType"), "my_type");
}

#[test]
fn pascal_to_snake_rdfa() {
    assert_eq!(pascal_to_snake("Rdfa"), "rdfa");
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
