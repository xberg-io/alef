use std::collections::{HashMap, HashSet};

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::backends::zig::gen_bindings::functions::emit_function;
use crate::backends::zig::gen_bindings::opaque_handles::{OpaqueHandleContext, emit_opaque_handle};
use crate::core::ir::{CoreWrapper, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

const PREFIX: &str = "sample";

/// The substring every generated reference to a presence companion contains. Parity assertions
/// search the *rendered Zig*, not the eligibility predicate, so an emitter that forgets to
/// consult the predicate fails them instead of agreeing with itself. ~keep
const COMPANION_MARKER: &str = "_has_result";

fn optional(inner: TypeRef) -> TypeRef {
    TypeRef::Optional(Box::new(inner))
}

fn i64_option() -> TypeRef {
    optional(TypeRef::Primitive(PrimitiveType::I64))
}

fn free_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn method_def(name: &str, return_type: TypeRef, receiver: Option<ReceiverKind>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver,
        ..Default::default()
    }
}

fn scalar_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: CoreWrapper::None,
    }
}

fn render_free_function(func: &FunctionDef) -> String {
    let mut out = String::new();
    emit_function(
        func,
        PREFIX,
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &mut out,
    );
    out
}

fn render_method(method: &MethodDef) -> String {
    let ty = TypeDef {
        name: "Settings".to_string(),
        rust_path: "sample::Settings".to_string(),
        is_opaque: true,
        methods: vec![method.clone()],
        ..TypeDef::default()
    };
    let mut out = String::new();
    emit_opaque_handle(
        &ty,
        PREFIX,
        &OpaqueHandleContext {
            declared_errors: &[],
            struct_names: &HashSet::new(),
            streaming_item_types: &HashMap::new(),
            enum_names: &HashSet::new(),
        },
        &mut out,
    );
    out
}

/// The comma-separated argument text of the first `call_prefix(...)` occurrence.
fn call_arguments<'a>(generated: &'a str, call_prefix: &str) -> &'a str {
    let start = generated
        .find(call_prefix)
        .unwrap_or_else(|| panic!("no `{call_prefix}` call in:\n{generated}"))
        + call_prefix.len();
    let end = start
        + generated[start..]
            .find(')')
            .unwrap_or_else(|| panic!("unterminated `{call_prefix}` call in:\n{generated}"));
    &generated[start..end]
}

/// Replaces the deleted `optional_primitive_return_stays_a_bare_passthrough`, which asserted the
/// defect: `unwrap_return_expr` returning a bare `_result` for an `Option<i64>` was called a
/// "positive control" when it was in fact the whole bug — Zig coerces `i64` into `?i64` as
/// non-null, so every `None` reached the caller as `0`.
///
/// The passthrough itself is still correct and is still asserted here, because absence is now
/// answered out of band by the companion; what changed is that the wrapper must consult the
/// companion first. Asserting both halves at the rendered-wrapper level is what makes the pair
/// meaningful — the old test could not tell a wired wrapper from an unwired one. ~keep
#[test]
fn should_report_absent_as_null_and_keep_a_zero_valued_result_present() {
    let generated = render_free_function(&free_function("port", i64_option()));

    assert!(
        generated.contains("pub fn port() ?i64 {"),
        "expected a nullable Zig return type; got:\n{generated}"
    );
    assert!(
        generated.contains("    if (c.sample_port_has_result() != 1) {\n        return null;\n    }\n"),
        "absence must come from the companion, not from the returned value; got:\n{generated}"
    );
    assert!(
        generated.contains("    return _result;\n"),
        "a present result, zero included, must be returned unchanged; got:\n{generated}"
    );
}

/// The companion clears the FFI crate's last-error slot on entry. Calling it after the primary
/// would erase an error the primary had just recorded. ~keep
#[test]
fn should_call_the_companion_before_the_primary_c_call() {
    let generated = render_free_function(&free_function("port", i64_option()));

    let companion_at = generated.find("c.sample_port_has_result(").expect("companion call");
    let primary_at = generated.find("const _result = c.sample_port(").expect("primary call");
    assert!(
        companion_at < primary_at,
        "the presence gate must run before the primary call; got:\n{generated}"
    );
}

#[test]
fn should_surface_a_companion_failure_as_the_wrappers_error_when_it_is_fallible() {
    let mut func = free_function("port", i64_option());
    func.error_type = Some("sample::Error".to_string());

    let generated = render_free_function(&func);

    let gate_at = generated
        .find("if (c.sample_port_has_result() != 1) {")
        .expect("presence gate");
    let after_gate = &generated[gate_at..];
    assert!(
        after_gate
            .starts_with("if (c.sample_port_has_result() != 1) {\n        if (c.sample_last_error_code() != 0) {\n"),
        "a fallible wrapper must report the companion's own failure rather than absence; \
         got:\n{generated}"
    );
}

/// The companion's C signature *is* the primary export's parameter list, receiver included.
#[test]
fn should_pass_the_same_arguments_to_the_companion_as_to_the_primary_call() {
    let mut timeout = method_def("timeout", i64_option(), Some(ReceiverKind::Ref));
    timeout.params = vec![scalar_param("scale", TypeRef::Primitive(PrimitiveType::U32))];

    let generated = render_method(&timeout);

    assert_eq!(
        call_arguments(&generated, "c.sample_settings_timeout_has_result("),
        call_arguments(&generated, "const _result = c.sample_settings_timeout("),
        "the companion and the primary must pass the same arguments; got:\n{generated}"
    );
}

#[test]
fn should_not_call_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render_free_function(&free_function("label", optional(TypeRef::String)));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
}

#[test]
fn should_not_call_a_companion_for_an_owned_receiver_because_ffi_exports_none() {
    let consumed = method_def("into_timeout", i64_option(), Some(ReceiverKind::Owned));
    assert!(
        !result_presence_companion_exists(&consumed.return_type, consumed.receiver.as_ref()),
        "the FFI backend must not export a companion for an owned receiver"
    );

    let generated = render_method(&consumed);

    assert!(
        !generated.contains(COMPANION_MARKER),
        "an owned receiver's first call consumes the handle, so no companion exists; got:\n{generated}"
    );
}

/// Zig's decision to call `{fn}_has_result` must equal the FFI backend's decision to export it,
/// for every shape and receiver. `@cImport` resolves externs at comptime, so calling a symbol the
/// FFI crate never exported is a build error in the consumer's package rather than a wrong value.
/// This compares the rendered Zig against the authority instead of restating the rule. ~keep
#[test]
fn zig_calls_a_companion_exactly_when_the_ffi_backend_exports_one() {
    let shapes: Vec<TypeRef> = vec![
        i64_option(),
        optional(TypeRef::Primitive(PrimitiveType::U64)),
        optional(TypeRef::Primitive(PrimitiveType::Bool)),
        optional(TypeRef::Primitive(PrimitiveType::F64)),
        optional(TypeRef::Duration),
        optional(TypeRef::String),
        optional(TypeRef::Path),
        optional(TypeRef::Json),
        optional(TypeRef::Bytes),
        optional(TypeRef::Named("Settings".to_string())),
        optional(TypeRef::Vec(Box::new(TypeRef::String))),
        TypeRef::Primitive(PrimitiveType::I64),
        TypeRef::String,
        TypeRef::Unit,
    ];
    let receivers = [
        Some(ReceiverKind::Ref),
        Some(ReceiverKind::RefMut),
        Some(ReceiverKind::Owned),
    ];

    for shape in &shapes {
        let generated = render_free_function(&free_function("probe", shape.clone()));
        assert_eq!(
            generated.contains(COMPANION_MARKER),
            result_presence_companion_exists(shape, None),
            "free function returning {shape:?} disagreed with the FFI backend; got:\n{generated}"
        );

        for receiver in &receivers {
            let generated = render_method(&method_def("probe", shape.clone(), receiver.clone()));
            assert_eq!(
                generated.contains(COMPANION_MARKER),
                result_presence_companion_exists(shape, receiver.as_ref()),
                "method returning {shape:?} with receiver {receiver:?} disagreed with the FFI \
                 backend; got:\n{generated}"
            );
        }
    }
}
