use super::{RustlerBackend, test_api, test_config};
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn client(is_clone: bool) -> TypeDef {
    TypeDef {
        name: "Client".to_string(),
        rust_path: "my_lib::Client".to_string(),
        original_rust_path: "my_lib::Client".to_string(),
        is_opaque: true,
        is_clone,
        methods: vec![MethodDef {
            name: "chat".to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: true,
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated_native(is_clone: bool) -> String {
    let mut api: ApiSurface = test_api();
    api.types = vec![client(is_clone)];
    RustlerBackend
        .generate_bindings(&api, &test_config())
        .expect("native binding generation must succeed")
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("expected native lib.rs")
        .content
        .clone()
}

/// A `&self` async method on a `Clone` opaque type must drop the `RwLock` read guard before the
/// future is awaited. `Ok(...)` is not enough: the guard has to be out of the expression that
/// spans `.await`, or `clippy::await_holding_lock` fails under `-D warnings` in every consumer
/// and a writer blocks for the whole request.
#[test]
fn cloneable_async_ref_method_clones_the_handle_out_of_the_guard() {
    let content = generated_native(true);

    assert!(
        content.contains("let __alef_inner = resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone();"),
        "the read guard must be released before the async block:\n{content}"
    );
    assert!(
        content.contains("__alef_inner.chat()"),
        "the future must run against the cloned handle:\n{content}"
    );
    assert!(
        !content.contains("resource.inner.read().unwrap_or_else(|e| e.into_inner()).chat()"),
        "the guard must not be held across `.await`:\n{content}"
    );
    assert!(
        !content.contains("await_holding_lock"),
        "a cloneable type needs no lint allowance:\n{content}"
    );
}

/// A non-`Clone` opaque type cannot be cloned out, so the guard stays and the generated function
/// carries a narrow lint allowance. This is the documented fallback, not the preferred shape.
#[test]
fn non_cloneable_async_ref_method_keeps_the_guard_with_a_narrow_allowance() {
    let content = generated_native(false);

    assert!(
        content.contains("#[allow(clippy::await_holding_lock)]"),
        "a non-Clone receiver must allow the lint where it is unavoidable:\n{content}"
    );
    assert!(
        content.contains("resource.inner.read().unwrap_or_else(|e| e.into_inner()).chat()"),
        "the non-Clone fallback must keep the original call shape:\n{content}"
    );
    assert!(
        !content.contains("let __alef_inner"),
        "a non-Clone receiver must not attempt to clone out of the guard:\n{content}"
    );
}
