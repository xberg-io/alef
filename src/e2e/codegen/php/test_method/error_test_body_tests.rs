use super::render_error_test_body;
use crate::core::ir::{ErrorDef, ErrorVariant};
use crate::e2e::fixture::{Assertion, Fixture};

fn fixture_with_declared_error(value: &str) -> Fixture {
    Fixture {
        id: "declares_error".to_string(),
        assertions: vec![Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn coded_error_def(variant_name: &str) -> ErrorDef {
    ErrorDef {
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: variant_name.to_string(),
            error_code: Some(100),
            is_unit: true,
            ..ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn unavailable_variant_still_asserts_an_exception_with_phpunit() {
    let fixture = fixture_with_declared_error("Authentication");
    let errors = vec![coded_error_def("Authentication")];
    let body = render_error_test_body(&[], "Client::create()", &fixture, &errors);
    assert!(body.contains("$this->expectException(\\Exception::class);"), "{body}");
    assert!(
        !body.contains("catch ("),
        "framework failures must not be caught: {body}"
    );
    assert!(body.contains("Client::create();"), "the actual call must run: {body}");
}

#[test]
fn literal_error_checks_are_outside_the_catch_that_observes_the_call() {
    let fixture = fixture_with_declared_error("Expected an exception to be thrown");
    let body = render_error_test_body(&[], "Client::create()", &fixture, &[]);
    let catch_end = body.find("        }\n").expect("catch must end before assertions");
    let assertion = body
        .find("$this->assertInstanceOf(\\Exception::class, $caughtException)")
        .expect("an actual exception must be asserted");
    assert!(assertion > catch_end, "{body}");
    assert!(
        !body.contains("$this->fail("),
        "do not synthesize an exception the fixture can match: {body}"
    );
}

#[test]
fn no_declared_value_is_byte_identical_to_expect_exception() {
    let fixture = Fixture {
        id: "no_error".to_string(),
        ..Fixture::default()
    };
    let body = render_error_test_body(
        &["$options = new Options();".to_string()],
        "Client::create($options)",
        &fixture,
        &[],
    );
    assert_eq!(
        body,
        "        $this->expectException(\\Exception::class);\n        $options = new Options();\n        Client::create($options);"
    );
}

/// With no `errors` IR supplied, a value cannot be recognised as a known variant name, so it
/// renders exactly like a message-style value always did before this fix.
#[test]
fn declared_value_adds_message_or_class_name_check() {
    let fixture = fixture_with_declared_error("BadRequest");
    let body = render_error_test_body(&[], "Client::create()", &fixture, &[]);
    assert_eq!(
        body,
        "        $caughtException = null;\n        try {\n            Client::create();\n        } catch (\\Exception $e) {\n            $caughtException = $e;\n        }\n        $this->assertInstanceOf(\\Exception::class, $caughtException);\n        $this->assertTrue(preg_match('/BadRequest/', $caughtException->getMessage()) === 1 || preg_match('/BadRequest/', get_class($caughtException)) === 1, 'expected exception message or class name to match BadRequest');"
    );
}

#[test]
fn declared_value_with_regex_metacharacters_is_escaped() {
    let fixture = fixture_with_declared_error("field.name[0]");
    let body = render_error_test_body(&[], "Client::create()", &fixture, &[]);
    assert!(
        body.contains("'/field\\\\.name\\\\[0\\\\]/'"),
        "expected escaped PCRE literal, got: {body}"
    );
}

/// Regression test for a real bug: embedding the already-quoted PCRE literal
/// (`'/max_depth/'`) directly inside the message's own single-quoted string
/// closes that string early and leaves a bare `/max_depth/` followed by `''`
/// — a PHP syntax error. Assert the message argument is exactly one balanced
/// single-quoted PHP string, not merely that it contains the right substrings
/// (string-content assertions alone did not catch this).
#[test]
fn declared_value_message_argument_is_a_well_formed_php_single_quoted_literal() {
    let fixture = fixture_with_declared_error("max_depth");
    let body = render_error_test_body(&[], "Client::create()", &fixture, &[]);
    let message_start = body
        .find("'expected exception message or class name to match")
        .expect("message argument must be present");
    let message = &body[message_start..];
    // The message argument runs up to the closing `)` of assertTrue(...); — find
    // the terminating `'` that precedes it.
    let message_end = message
        .find("');")
        .expect("message argument must be closed and followed by ');'");
    let literal = &message[..=message_end];
    // Strip PHP's `\'` escape sequences before counting quote boundaries, so an
    // escaped quote inside the literal doesn't look like a premature terminator.
    let quote_count = literal.replace("\\'", "").matches('\'').count();
    assert_eq!(
        quote_count, 2,
        "message argument must be a single balanced single-quoted PHP string, got: {literal}"
    );
    assert!(
        literal.contains("max_depth"),
        "failure message should still name the expected pattern: {literal}"
    );
    assert!(
        !literal.contains("'/max_depth/'"),
        "must not embed the pre-quoted PCRE literal inside the message string: {literal}"
    );
}

#[test]
fn declared_value_with_single_quote_is_escaped_in_message() {
    let fixture = fixture_with_declared_error("it's bad");
    let body = render_error_test_body(&[], "Client::create()", &fixture, &[]);
    assert!(
        body.contains("match it\\'s bad')"),
        "expected escaped single quote in message, got: {body}"
    );
}

/// The defect this fix closes: a declared value that names a real `ErrorVariant` — PHP's
/// generated binding has exactly one exception class for the whole extension — must render
/// the registered skip, not a `preg_match` comparison that can never pass.
#[test]
fn declared_value_naming_a_known_variant_renders_the_registered_skip_instead_of_an_assertion() {
    let fixture = fixture_with_declared_error("Authentication");
    let errors = vec![coded_error_def("Authentication")];
    let body = render_error_test_body(&[], "Client::create()", &fixture, &errors);
    assert_eq!(
        body,
        "        // skipped: declared error variant 'Authentication' not yet preserved as a distinct identity by this backend's generator\n        $this->expectException(\\Exception::class);\n        Client::create();"
    );
    assert!(
        !body.contains("assertTrue(preg_match"),
        "must not render an assertion that can never pass, got: {body}"
    );
}
