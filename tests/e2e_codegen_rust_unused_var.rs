//! Regression test: auxiliary metadata unwrap bindings must be `_`-prefixed
//! so they do not trip `-D unused_variables` when no assertion references them.
//!
//! Reproduces a generated Rust e2e CI failure: the Rust e2e
//! emitter generated `let metadata_output_format = …` but never referenced it
//! in the following assertions, causing a compiler error under `-D warnings`.

use std::collections::{HashMap, HashSet};

use alef::e2e::field_access::FieldResolver;

/// Build a FieldResolver that marks `metadata.output_format` as an optional field.
fn make_resolver_with_optional_output_format() -> FieldResolver {
    let mut aliases: HashMap<String, String> = HashMap::new();
    aliases.insert(
        "metadata.output_format".to_string(),
        "metadata.output_format".to_string(),
    );

    let mut optional: HashSet<String> = HashSet::new();
    optional.insert("metadata.output_format".to_string());

    let result_fields: HashSet<String> = HashSet::new();
    let array: HashSet<String> = HashSet::new();
    let method_calls: HashSet<String> = HashSet::new();

    FieldResolver::new(&aliases, &optional, &result_fields, &array, &method_calls)
}

/// The generated binding name must start with `_` to suppress `-D unused_variables`
/// when the assertion for that field is skipped by the renderer.
#[test]
fn rust_unwrap_binding_is_underscore_prefixed() {
    let resolver = make_resolver_with_optional_output_format();
    let (binding, local_var) = resolver
        .rust_unwrap_binding("metadata.output_format", "result")
        .expect("metadata.output_format is optional, binding must be produced");

    assert!(
        local_var.starts_with('_'),
        "local_var must start with `_` to suppress unused-variable warnings; got: {local_var:?}"
    );
    assert!(
        binding.starts_with(&format!("let {local_var} =")),
        "binding declaration must use the same `_`-prefixed name; got: {binding:?}"
    );
    assert!(
        binding.contains("as_ref().map(|v| v.to_string()).unwrap_or_default()"),
        "binding must use Display-based unwrap; got: {binding:?}"
    );
}

/// When the binding IS referenced in an assertion, the `_`-prefixed name is still
/// valid — `_foo` is fully accessible in Rust (the prefix only suppresses the warning).
/// Verify the returned local_var is consistent with the binding declaration name.
#[test]
fn rust_unwrap_binding_local_var_matches_binding_declaration() {
    let resolver = make_resolver_with_optional_output_format();
    let (binding, local_var) = resolver
        .rust_unwrap_binding("metadata.output_format", "result")
        .expect("metadata.output_format is optional");

    let expected_decl = format!("let {local_var} =");
    assert!(
        binding.starts_with(&expected_decl),
        "binding declaration `{binding}` must start with `{expected_decl}`"
    );
}

/// Double-underscore collapsing still works with the `_` prefix:
/// a path like `json_ld[].name` must yield `_json_ld_name`, not `_json_ld__name`.
#[test]
fn rust_unwrap_binding_collapses_double_underscore_with_prefix() {
    let mut aliases: HashMap<String, String> = HashMap::new();
    aliases.insert("json_ld.name".to_string(), "json_ld[].name".to_string());

    let mut optional: HashSet<String> = HashSet::new();
    optional.insert("json_ld[].name".to_string());

    let mut array: HashSet<String> = HashSet::new();
    array.insert("json_ld".to_string());

    let result_fields: HashSet<String> = HashSet::new();
    let method_calls: HashSet<String> = HashSet::new();
    let resolver = FieldResolver::new(&aliases, &optional, &result_fields, &array, &method_calls);

    let (_binding, var) = resolver
        .rust_unwrap_binding("json_ld.name", "result")
        .expect("json_ld.name is optional");

    assert_eq!(
        var, "_json_ld_name",
        "double underscores must be collapsed and result prefixed with `_`; got: {var:?}"
    );
    assert!(
        !var.contains("__"),
        "collapsed local_var must not contain double underscores; got: {var:?}"
    );
}
