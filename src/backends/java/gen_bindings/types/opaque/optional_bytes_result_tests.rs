//! A Java method returning `Option<Vec<u8>>` must wrap its `byte[]` in an `Optional`.
//!
//! `stream_method_bytes_result.jinja` branches on a single context key, `optional`, but
//! `emit_bytes_result` passed only the pre-rendered `empty_return`/`success_return` strings that
//! two sibling templates consume. minijinja reads the undefined `optional` as falsy, so the wrap
//! was dead code: the method was *declared* `Optional<byte[]>` and *returned* a bare `byte[]`,
//! which does not compile. Nothing caught it — the C# sibling has
//! `csharp/gen_bindings/methods/bytes_out_param_tests.rs`, Java had no equivalent, and every Java
//! CI leg but the musl one builds the native library without ever compiling the Java sources.
//! These pin both arms of the branch. ~keep

use super::*;
use crate::core::ir::{MethodDef, ReceiverKind, TypeRef};

fn bytes_method(return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: "sample_bytes".into(),
        return_type,
        error_type: Some("SampleError".into()),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn render(return_type: TypeRef) -> String {
    let mut out = String::new();
    gen_instance_method(
        &mut out,
        &bytes_method(return_type),
        "sample",
        "registry",
        "SampleRs",
        &AHashSet::new(),
        &AHashSet::new(),
        &AHashSet::new(),
    );
    out
}

#[test]
fn optional_bytes_return_is_wrapped_in_an_optional() {
    let generated = render(TypeRef::Optional(Box::new(TypeRef::Bytes)));
    assert!(
        generated.contains("java.util.Optional<byte[]>"),
        "expected an Optional<byte[]> return type, got:\n{generated}"
    );
    assert!(
        generated.contains("java.util.Optional.of(result)") && generated.contains("java.util.Optional.empty()"),
        "an Optional<byte[]> method must wrap both arms, got:\n{generated}"
    );
    assert!(
        !generated.contains("            return result;"),
        "a bare `return result;` does not compile against Optional<byte[]>, got:\n{generated}"
    );
}

#[test]
fn bare_bytes_return_is_not_wrapped() {
    let generated = render(TypeRef::Bytes);
    assert!(
        generated.contains("return result;"),
        "expected a bare byte[] return, got:\n{generated}"
    );
    assert!(
        !generated.contains("java.util.Optional.of(result)"),
        "a byte[] method must not wrap its result, got:\n{generated}"
    );
}
