//! An `iterate` presentation operation with an empty `fields` list — a legitimate, reachable
//! shape (`presentation::authored_operation_validation_tests::
//! an_authored_iterate_drops_only_the_unknown_per_item_field` proves it survives resolution) —
//! must still render a syntactically valid loop.
//!
//! `python/snippet_body.py.jinja`'s `for {{ item }} in ...: {% for field in operation.fields %}
//! print({{ field }}) {% endfor %}` emitted nothing at all for the loop body when `fields` was
//! empty. Unlike Go/TypeScript/Rust, where an empty `{ }` block is valid syntax, Python requires
//! an indented statement after a `:` — a published snippet with no per-item fields was an
//! `IndentationError`, not merely a useless loop. The fix mirrors the fallback
//! `go/snippet_body.jinja` already has for the same shape (there, to dodge Go's "declared and not
//! used" loop-variable error): when `fields` is empty, print the raw loop item instead.

use crate::e2e::codegen::presentation::PresentationOperation;

fn iterate_operation(fields: Vec<&str>) -> PresentationOperation {
    let fields: Vec<String> = fields.into_iter().map(str::to_string).collect();
    PresentationOperation {
        kind: "iterate",
        expression: "result.items".to_string(),
        item: "item".to_string(),
        field_displays: vec![false; fields.len()],
        fields,
        optional: false,
        display: false,
        destructure_source: String::new(),
        destructure_item: String::new(),
        shown_optional: false,
        field_optionals: Vec::new(),
        guard_binding: String::new(),
        guard_source: String::new(),
        guard_condition: String::new(),
    }
}

fn render(operation: PresentationOperation) -> String {
    crate::e2e::template_env::render(
        "python/snippet_body.py.jinja",
        minijinja::context! {
            imports => Vec::<String>::new(),
            body => vec!["result = client.list_items()".to_string()],
            is_async => false,
            presentation => vec![operation],
            expects_error => false,
            error_type => "Error",
            typed_error_type => Option::<String>::None,
            result_var => "result",
            returns_void => false,
        },
    )
}

/// REPRODUCTION: an empty-fields `iterate` must render a non-empty loop body — Python has no
/// syntax for an empty `for` block.
#[test]
fn an_iterate_with_no_fields_prints_the_raw_item_instead_of_an_empty_block() {
    let rendered = render(iterate_operation(vec![]));

    assert!(
        rendered.contains("for item in result.items:\n        print(item)"),
        "the loop body must be the raw item, immediately after the `for` line:\n{rendered}"
    );
}

/// CONTROL: a populated `fields` list must keep printing exactly those fields — no `print(item)`
/// fallback line grafted on alongside them.
#[test]
fn an_iterate_with_fields_prints_only_those_fields() {
    let rendered = render(iterate_operation(vec!["item.value"]));

    assert!(
        rendered.contains("for item in result.items:\n        print(item.value)"),
        "the declared field must still be printed:\n{rendered}"
    );
    assert!(
        !rendered.contains("print(item)\n"),
        "the raw-item fallback must not appear when real fields were rendered:\n{rendered}"
    );
}
