use super::*;
use crate::core::ir::{NewtypeWrapper, NewtypeWrapperMetadata, ParamDef};

fn transparent_string_wrapper() -> String {
    NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "sample_crate::SecretString",
        "from",
        "into_inner",
        vec![],
    )])
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn opaque_type(name: &str, methods: Vec<crate::core::ir::MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{name}"),
        is_opaque: true,
        methods,
        ..Default::default()
    }
}

fn async_method(name: &str, receiver: ReceiverKind, error_type: Option<&str>) -> crate::core::ir::MethodDef {
    crate::core::ir::MethodDef {
        name: name.to_string(),
        params: vec![param("req", TypeRef::Named("Request".to_string()))],
        return_type: TypeRef::Named("Response".to_string()),
        error_type: error_type.map(str::to_string),
        is_async: true,
        receiver: Some(receiver),
        ..Default::default()
    }
}

/// An async method must be a real `async fn` (the matching extern declaration is `async fn`,
/// which is what makes the Swift side suspend instead of parking a thread). Its work is spawned
/// onto the process-wide runtime with a `'static` receiver, only the `JoinHandle` is awaited,
/// and a failed join becomes `Err(String)` rather than unwinding.
#[test]
fn async_method_shim_is_an_async_fn_that_spawns_and_awaits_the_join_handle() {
    let ty = opaque_type(
        "Client",
        vec![
            async_method("chat", ReceiverKind::Ref, Some("ClientError")),
            async_method("infallible", ReceiverKind::Ref, None),
        ],
    );

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    for name in ["client_chat", "client_infallible"] {
        assert!(
            out.contains(&format!(
                "pub async fn {name}(client: &Client, req: Request) -> Result<Response, String>"
            )),
            "`{name}` must be an `async fn` with a Result return (even when infallible, so a \
                 JoinError has somewhere to land), got:\n{out}"
        );
    }
    assert!(
        !out.contains("block_on"),
        "an async shim must never block a thread on the runtime, got:\n{out}"
    );
    assert!(
        out.contains("crate::__alef_tokio_runtime().spawn(async move {"),
        "the work must be spawned onto the large-stack runtime, got:\n{out}"
    );
    assert!(
        out.contains("let client: &'static Client = unsafe { &*(client as *const Client) };"),
        "spawn needs a 'static receiver, got:\n{out}"
    );
    assert!(
        out.contains("__alef_task.await.unwrap_or_else(|__alef_join_error|"),
        "a join failure must surface as Err(String), got:\n{out}"
    );
}

#[test]
fn async_method_shim_extends_a_mut_receiver_as_static_mut() {
    let ty = opaque_type(
        "Client",
        vec![async_method("chat", ReceiverKind::RefMut, Some("ClientError"))],
    );

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        out.contains("pub async fn client_chat(client: &mut Client,"),
        "got:\n{out}"
    );
    assert!(
        out.contains("let client: &'static mut Client = unsafe { &mut *(client as *mut Client) };"),
        "got:\n{out}"
    );
}

#[test]
fn sync_method_shim_stays_a_plain_fn() {
    let mut method = async_method("chat", ReceiverKind::Ref, Some("ClientError"));
    method.is_async = false;
    let ty = opaque_type("Client", vec![method]);

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(out.contains("pub fn client_chat("), "got:\n{out}");
    assert!(!out.contains("async") && !out.contains("spawn"), "got:\n{out}");
}

#[test]
fn transparent_string_method_param_and_return_use_explicit_operations() {
    let mut value = param("value", TypeRef::String);
    value.newtype_wrapper = Some(transparent_string_wrapper());
    let method = crate::core::ir::MethodDef {
        name: "replace_secret".to_string(),
        params: vec![value],
        return_type: TypeRef::String,
        return_newtype_wrapper: Some(transparent_string_wrapper()),
        receiver: Some(ReceiverKind::RefMut),
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        out.contains("replace_secret(sample_crate::SecretString::from(value))"),
        "{out}"
    );
    assert!(out.contains(".into_inner()"), "{out}");
    assert!(!out.contains(".to_string()"), "{out}");
}

#[test]
fn first_class_async_transparent_methods_are_explicitly_rejected() {
    let infallible = crate::core::ir::MethodDef {
        name: "async_secret".to_string(),
        return_type: TypeRef::String,
        return_newtype_wrapper: Some(transparent_string_wrapper()),
        receiver: Some(ReceiverKind::Ref),
        is_async: true,
        ..Default::default()
    };
    let fallible = crate::core::ir::MethodDef {
        name: "async_fallible_secret".to_string(),
        return_type: TypeRef::String,
        return_newtype_wrapper: Some(transparent_string_wrapper()),
        receiver: Some(ReceiverKind::Ref),
        is_async: true,
        error_type: Some("SecretError".to_string()),
        ..Default::default()
    };
    let ty = TypeDef {
        name: "Secrets".to_string(),
        rust_path: "sample_crate::Secrets".to_string(),
        methods: vec![infallible, fallible],
        ..Default::default()
    };

    let output = emit_first_class_dto_method_wrappers(&ty, "sample_crate", &HashMap::new(), &HashSet::new());

    assert!(
        output.contains("cannot safely bridge async first-class DTO method `async_secret`"),
        "{output}"
    );
    assert!(
        output.contains("cannot safely bridge async first-class DTO method `async_fallible_secret`"),
        "{output}"
    );
    assert!(!output.contains("block_on(") && !output.contains(".await"), "{output}");
}

#[test]
fn first_class_async_without_transparent_wrappers_keeps_existing_path() {
    let method = crate::core::ir::MethodDef {
        name: "plain_async".to_string(),
        return_type: TypeRef::String,
        receiver: Some(ReceiverKind::Ref),
        is_async: true,
        ..Default::default()
    };
    let ty = TypeDef {
        name: "Plain".to_string(),
        rust_path: "sample_crate::Plain".to_string(),
        methods: vec![method],
        ..Default::default()
    };

    let output = emit_first_class_dto_method_wrappers(&ty, "sample_crate", &HashMap::new(), &HashSet::new());

    assert!(
        output.contains("block_on(async { __self.plain_async().await })"),
        "{output}"
    );
    assert!(!output.contains("compile_error!"), "{output}");
}

/// Same defect shape and fix as `shims::tests::infallible_function_with_direct_enum_param_*`,
/// but for the instance-method wrapper path: an unrecognised wire string used to `panic!`
/// inside the reverse-conversion helper, which is UB once it unwinds across the swift-bridge
/// FFI boundary. An infallible method (no `error_type`) with a unit-enum param must have its
/// wrapper's return type forced to `Result<_, String>` so the `?` in the conversion has
/// somewhere to go, and the success path wrapped in `Ok(..)`.
#[test]
fn infallible_method_with_enum_param_gets_forced_result_return_and_no_panic() {
    let method = crate::core::ir::MethodDef {
        name: "set_mode".to_string(),
        params: vec![param("mode", TypeRef::Named("Mode".to_string()))],
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::RefMut),
        error_type: None,
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::from(["Mode"]);
    let unit_enum_names = enum_names.clone();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::new();

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(
        out.contains("-> Result<(), String>"),
        "an infallible method with a fallible enum param conversion must have its wrapper \
             return type forced to Result so the conversion error can propagate, got:\n{out}"
    );
    assert!(
        out.contains(&format!("{}(&mode)?", enum_from_string_fn_name("Mode"))),
        "expected the reverse-conversion call to be `?`-propagated, got:\n{out}"
    );
    assert!(
        out.contains("Ok("),
        "the originally-infallible success path must be wrapped in Ok(..) once the \
             wrapper's return type is forced to Result, got:\n{out}"
    );
    assert!(
        !out.contains("panic!"),
        "must not panic across the FFI boundary, got:\n{out}"
    );
    assert!(
        !out.contains(".expect(\"valid"),
        "must not paper over the fallible conversion with .expect(..) either, got:\n{out}"
    );
}

/// A method that is already fallible (`error_type` set) must still `?`-propagate the enum
/// conversion, without double-wrapping the return type.
#[test]
fn fallible_method_with_enum_param_still_propagates_conversion_error() {
    let method = crate::core::ir::MethodDef {
        name: "set_mode".to_string(),
        params: vec![param("mode", TypeRef::Named("Mode".to_string()))],
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::RefMut),
        error_type: Some("ClientError".to_string()),
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::from(["Mode"]);
    let unit_enum_names = enum_names.clone();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::new();

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(out.contains("-> Result<(), String>"), "got:\n{out}");
    assert!(
        out.contains(&format!("{}(&mode)?", enum_from_string_fn_name("Mode"))),
        "got:\n{out}"
    );
    assert!(!out.contains("panic!"), "got:\n{out}");
}

/// Regression test for the alef CI `generated-output-gate` panic: swift-bridge-ir 0.1.59's
/// `BridgedType::to_alpha_numeric_underscore_name` (`bridged_type.rs:1986`) has a match arm
/// for every Rust integer primitive width except `u64`/`i64`; those two fall through to an
/// unconditional `todo!()`. Every `Result<Ok, String>` alef emits reaches that function
/// (see `result_ok_needs_json_bridge_with_handles`'s doc comment for why), so declaring
/// `Result<u64, String>` on a fallible method panicked `alef generate`'s own swift build,
/// not just a downstream consumer's. Bridging the ok type through JSON avoids the
/// panicking match arm entirely.
#[test]
fn fallible_method_returning_u64_bridges_through_json_not_a_bare_u64() {
    let method = crate::core::ir::MethodDef {
        name: "count".to_string(),
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
        receiver: Some(ReceiverKind::Ref),
        error_type: Some("ClientError".to_string()),
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::new();
    let unit_enum_names = enum_names.clone();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::new();

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(
        out.contains("-> Result<String, String>"),
        "u64 Ok type must be bridged through JSON to dodge swift-bridge-ir's todo!() on \
             u64/i64, got:\n{out}"
    );
    assert!(
        !out.contains("Result<u64, String>"),
        "must never declare the panic-triggering Result<u64, String>, got:\n{out}"
    );
    assert!(
        out.contains("serde_json::to_string(&v)"),
        "the u64 value must be JSON-serialized to match the declared String Ok type, got:\n{out}"
    );
}

/// The u64/i64 JSON-bridge in `fallible_method_returning_u64_bridges_through_json_not_a_bare_u64`
/// is scoped to the `Result` position only: a bare, infallible `u64` return never reaches
/// swift-bridge-ir's panicking path and must keep its native type.
#[test]
fn infallible_method_returning_u64_keeps_native_type() {
    let method = crate::core::ir::MethodDef {
        name: "count".to_string(),
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
        receiver: Some(ReceiverKind::Ref),
        error_type: None,
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::new();
    let unit_enum_names = enum_names.clone();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::new();

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(
        out.contains("-> u64"),
        "an infallible u64 getter must keep its native type, got:\n{out}"
    );
    assert!(
        !out.contains("serde_json::to_string"),
        "an infallible u64 getter must not be JSON-bridged, got:\n{out}"
    );
}

/// `emit_type_method_shims` used to take a single enum set, and `gen_rust_crate::mod` handed it
/// `enum_names` (ALL enums) for a parameter named `unit_enum_names`. A data-carrying enum
/// parameter therefore reached the unit-enum branch and emitted a call to
/// `__alef_{enum}_from_swift_string` -- a helper `enums.rs` deliberately emits only for
/// fieldless enums (`E0425`) -- and, through `forces_fallible_enum_bridge`, forced the wrapper's
/// return type to `Result` while `extern_block::emit_extern_block_for_type_methods` (fed the
/// real `unit_enum_names`) declared the infallible one, an `E0308` signature mismatch. A tagged
/// enum parameter must instead cross as JSON, the route free-function parameters already take.
#[test]
fn data_carrying_enum_method_param_bridges_through_json_not_the_unit_enum_helper() {
    let method = crate::core::ir::MethodDef {
        name: "set_routing".to_string(),
        params: vec![param("routing", TypeRef::Named("Routing".to_string()))],
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::RefMut),
        error_type: None,
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::from(["Routing"]);
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::from([("Routing".to_string(), "sample_crate::Routing".to_string())]);

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(
        out.contains("routing: String"),
        "every enum crosses the bridge as a String, matching the extern declaration, got:\n{out}"
    );
    assert!(
        out.contains("::serde_json::from_str::<sample_crate::Routing>(&routing)"),
        "a data-carrying enum parameter must be deserialized from JSON, got:\n{out}"
    );
    assert!(
        !out.contains(&enum_from_string_fn_name("Routing")),
        "must not call the unit-enum-only from-string helper, which is never emitted for a \
             data-carrying enum, got:\n{out}"
    );
    assert!(
        !out.contains("-> Result<"),
        "a data-carrying enum parameter has no fallible from-string conversion to propagate, \
             so the wrapper must keep the infallible signature the extern block declares, got:\n{out}"
    );
}

/// Same defect, same fix, for the `Vec<Enum>` parameter shape: the vector branch also tested
/// the wrongly-supplied set and mapped every element through the unit-enum-only helper.
#[test]
fn data_carrying_enum_vec_method_param_bridges_each_element_through_json() {
    let method = crate::core::ir::MethodDef {
        name: "set_routes".to_string(),
        params: vec![param(
            "routes",
            TypeRef::Vec(Box::new(TypeRef::Named("Routing".to_string()))),
        )],
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::RefMut),
        error_type: None,
        ..Default::default()
    };
    let ty = opaque_type("Client", vec![method]);
    let enum_names = HashSet::from(["Routing"]);
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::new();
    let type_paths = HashMap::from([("Routing".to_string(), "sample_crate::Routing".to_string())]);

    let out = emit_type_method_shims(
        &ty,
        "sample_crate",
        &type_paths,
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
    );

    assert!(
        out.contains("routes: Vec<String>"),
        "a Vec of enums crosses as Vec<String>, matching the extern declaration, got:\n{out}"
    );
    assert!(
        out.contains("::serde_json::from_str::<sample_crate::Routing>(&s)"),
        "each element must be deserialized from JSON, got:\n{out}"
    );
    assert!(
        !out.contains(&enum_from_string_fn_name("Routing")),
        "must not call the unit-enum-only from-string helper, got:\n{out}"
    );
    assert!(
        !out.contains("-> Result<"),
        "no fallible conversion is involved, so the infallible signature must be kept, got:\n{out}"
    );
}
