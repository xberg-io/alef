//! An async core function or method is declared `async fn` in the `extern "Rust"` block so that
//! swift-bridge emits a Swift `async` function that suspends on a continuation. Declaring it as a
//! plain `fn` would make the matching `pub async fn` shim fail to type-check and, worse, make
//! the Swift call synchronous again. ~keep

use super::{emit_extern_block_for_functions, emit_extern_block_for_type_methods};
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::{BTreeSet, HashMap, HashSet};

fn function(is_async: bool) -> FunctionDef {
    FunctionDef {
        name: "fetch_status".to_string(),
        params: vec![ParamDef {
            name: "id".to_string(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::String,
        error_type: Some("SampleError".to_string()),
        is_async,
        ..FunctionDef::default()
    }
}

fn function_block(f: &FunctionDef) -> String {
    emit_extern_block_for_functions(
        std::slice::from_ref(f),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &BTreeSet::new(),
        &HashMap::new(),
        &ahash::AHashSet::default(),
    )
    .expect("emit_extern_block_for_functions")
}

fn client_with(method: MethodDef) -> TypeDef {
    TypeDef {
        name: "Client".to_string(),
        is_opaque: true,
        methods: vec![method],
        ..TypeDef::default()
    }
}

fn method(is_async: bool, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: "chat".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        error_type: error_type.map(str::to_string),
        is_async,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

fn method_block(m: MethodDef) -> String {
    emit_extern_block_for_type_methods(&client_with(m), &HashSet::new(), &HashSet::new(), &HashSet::new())
        .expect("emit_extern_block_for_type_methods")
}

#[test]
fn async_function_is_declared_async_fn() {
    let block = function_block(&function(true));
    assert!(
        block.contains("async fn fetch_status(id: String) -> Result<String, String>;"),
        "got:\n{block}"
    );
}

#[test]
fn sync_function_is_not_declared_async() {
    let block = function_block(&function(false));
    assert!(!block.contains("async"), "got:\n{block}");
    assert!(block.contains("fn fetch_status(id: String)"), "got:\n{block}");
}

#[test]
fn async_method_is_declared_async_fn_with_a_result_even_when_infallible() {
    let fallible = method_block(method(true, Some("SampleError")));
    assert!(
        fallible.contains("async fn client_chat(client: &Client) -> Result<String, String>;"),
        "got:\n{fallible}"
    );

    let infallible = method_block(method(true, None));
    assert!(
        infallible.contains("async fn client_chat(client: &Client) -> Result<String, String>;"),
        "the shim is forced fallible for the JoinError, so the declaration must agree; got:\n{infallible}"
    );
}

#[test]
fn sync_method_keeps_its_plain_declaration() {
    let block = method_block(method(false, None));
    assert!(!block.contains("async"), "got:\n{block}");
    assert!(
        block.contains("fn client_chat(client: &Client) -> String;"),
        "got:\n{block}"
    );
}

/// The template's `lstrip_blocks` setting strips the whitespace in front of a `{% if %}` tag at the
/// start of a line, which once pulled every declaration out of its `extern "Rust"` block. ~keep
#[test]
fn declarations_keep_their_indentation_inside_the_extern_block() {
    let sync = function_block(&function(false));
    assert!(
        sync.contains("\n        fn fetch_status(id: String)"),
        "sync declaration lost its indentation; got:\n{sync}"
    );
    let asynchronous = function_block(&function(true));
    assert!(
        asynchronous.contains("\n        async fn fetch_status(id: String)"),
        "async declaration lost its indentation; got:\n{asynchronous}"
    );
}
