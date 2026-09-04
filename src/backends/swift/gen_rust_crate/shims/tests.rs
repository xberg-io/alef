use super::*;
use crate::core::ir::{ParamDef, TypeRef};

fn param(name: &str, ty: TypeRef) -> ParamDef {
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn function(params: Vec<ParamDef>) -> FunctionDef {
    FunctionDef {
        name: "interact".to_string(),
        rust_path: "sample_crawler::interact".to_string(),
        original_rust_path: String::new(),
        params,
        return_type: TypeRef::Named("InteractionResult".to_string()),
        is_async: true,
        error_type: Some("CrawlError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn vec_enum_params_bridge_as_json_strings() {
    let f = function(vec![param(
        "actions",
        TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
    )]);
    let enum_names = HashSet::from(["PageAction"]);
    let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();

    assert!(is_bridgeable_fn(
        &f,
        &enum_names,
        &type_paths,
        &no_serde_names,
        &HashSet::new(),
        &handle_returned_types
    ));

    let capsule_types = std::collections::HashMap::new();
    let tagged_enum_names = HashSet::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };
    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(shim.contains("actions: Vec<String>"));
    assert!(
        shim.contains(&enum_from_string_fn_name("PageAction")),
        "expected a call into the reverse-conversion helper, not a From<String> impl (which \
         would be an orphan impl on the consumer's own enum type), got:\n{shim}"
    );
    assert!(!shim.contains("From<String>"));
    assert!(!shim.contains(".0"));
}

#[test]
fn direct_enum_params_bridge_as_from_string() {
    let f = function(vec![param("action", TypeRef::Named("PageAction".to_string()))]);
    let enum_names = HashSet::from(["PageAction"]);
    let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();

    let capsule_types = std::collections::HashMap::new();
    let tagged_enum_names = HashSet::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };
    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(shim.contains("action: String"));
    assert!(
        shim.contains(&enum_from_string_fn_name("PageAction")),
        "expected a call into the reverse-conversion helper, not a From<String> impl (which \
         would be an orphan impl on the consumer's own enum type), got:\n{shim}"
    );
    assert!(!shim.contains("From<String>"));
    assert!(!shim.contains("unimplemented!"));
}

/// An unrecognised wire string used to `panic!` inside the reverse-conversion helper --
/// undefined behaviour once that unwind crosses the swift-bridge FFI boundary. The helper now
/// returns `Result<_, String>`; when the wrapped core function is itself infallible (no
/// `error_type`), the shim's own return type must be forced to `Result<_, String>` purely so the
/// conversion's `?` has somewhere to propagate to -- otherwise the fix would just move the panic
/// from inside the helper to a `?` with no enclosing `Result`, which does not compile.
#[test]
fn infallible_function_with_direct_enum_param_gets_forced_result_return_and_no_panic() {
    let mut f = function(vec![param("action", TypeRef::Named("PageAction".to_string()))]);
    f.error_type = None;
    f.is_async = false;
    f.return_type = TypeRef::Unit;
    let enum_names = HashSet::from(["PageAction"]);
    let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let tagged_enum_names = HashSet::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };
    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(
        shim.contains("-> Result<(), String>"),
        "an infallible function with a fallible enum param conversion must have its shim \
         return type forced to Result so the conversion error can propagate, got:\n{shim}"
    );
    assert!(
        shim.contains(&format!("{}(&action)?", enum_from_string_fn_name("PageAction"))),
        "expected the reverse-conversion call to be `?`-propagated, got:\n{shim}"
    );
    assert!(
        shim.contains("Ok(())"),
        "the originally-infallible success path must be wrapped in Ok(..) once the shim's \
         return type is forced to Result, got:\n{shim}"
    );
    assert!(
        !shim.contains("panic!"),
        "must not panic across the FFI boundary, got:\n{shim}"
    );
    assert!(
        !shim.contains(".expect(\"valid"),
        "must not paper over the fallible conversion with .expect(..) either, got:\n{shim}"
    );
}

/// A `Vec<enum>` parameter on an infallible function is the same defect shape as the direct
/// case: any invalid element used to panic inside the per-element `.map(...)` closure.
#[test]
fn infallible_function_with_vec_enum_param_gets_forced_result_return_and_no_panic() {
    let mut f = function(vec![param(
        "actions",
        TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
    )]);
    f.error_type = None;
    f.return_type = TypeRef::Unit;
    let enum_names = HashSet::from(["PageAction"]);
    let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let tagged_enum_names = HashSet::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };
    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(shim.contains("-> Result<(), String>"), "got:\n{shim}");
    assert!(
        shim.contains("collect::<Result<Vec<_>, String>>()?"),
        "expected the per-element conversion to collect into a Result and `?`-propagate, got:\n{shim}"
    );
    assert!(
        !shim.contains("panic!"),
        "must not panic across the FFI boundary, got:\n{shim}"
    );
}

/// Data-carrying enums cross swift-bridge as JSON strings. A referenced enum parameter
/// therefore must be deserialized before it is borrowed for the source call; treating the
/// bridge `String` as an opaque wrapper emits `&param.0` and fails with E0609.
#[test]
fn referenced_tagged_enum_param_is_deserialized_before_source_call() {
    let mut output_format = param("output_format", TypeRef::Named("OutputFormat".to_string()));
    output_format.is_ref = true;
    let f = function(vec![
        param(
            "layout_enabled",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
        ),
        output_format,
    ]);
    let unit_enum_names = HashSet::new();
    let tagged_enum_names = HashSet::from(["OutputFormat"]);
    let type_paths = HashMap::from([("OutputFormat".to_string(), "sample_crawler::OutputFormat".to_string())]);
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = HashMap::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &unit_enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("output_format: String"),
        "tagged enums use the String bridge type:\n{shim}"
    );
    assert!(
        shim.contains(
            "&::serde_json::from_str::<sample_crawler::OutputFormat>(&output_format)\
             .expect(\"valid JSON for output_format\")"
        ),
        "the bridge string must be deserialized into the source enum before borrowing:\n{shim}"
    );
    assert!(
        !shim.contains("output_format.0"),
        "a bridge String has no opaque-wrapper field:\n{shim}"
    );
}

#[test]
fn vec_string_with_ref_inner_converts_to_slice_of_strs() {
    let mut param = param("names", TypeRef::Vec(Box::new(TypeRef::String)));
    param.is_ref = true;
    param.vec_inner_is_ref = true;

    let f = function(vec![param]);
    let type_paths = HashMap::new();
    let unit_enum_names = HashSet::new();
    let tagged_enum_names = HashSet::new();
    let no_serde_names = HashSet::new();
    let handle_returned_types = HashSet::new();

    assert!(is_bridgeable_fn(
        &f,
        &unit_enum_names,
        &type_paths,
        &no_serde_names,
        &tagged_enum_names,
        &handle_returned_types
    ));

    let capsule_types = std::collections::HashMap::new();
    let context = FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths: &type_paths,
        unit_enum_names: &unit_enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &ahash::AHashSet::default(),
    };
    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(shim.contains("names: Vec<String>"));
    assert!(shim.contains("&names.iter().map(|s| s.as_str()).collect::<Vec<_>>()"));
    assert!(shim.contains("sample_crawler::interact"));
}

// --- `&mut T` DTO writeback coverage (alef issue #380) -------------------------------------
//
// `fn tag_record(record: &mut Record)` used to render as
// `pub fn tag_record(mut record: Record) { probe_lib::tag_record(&mut record.0); }` --
// mutating the swift-bridge newtype and then dropping it without ever returning the
// update. See `crate::codegen::mut_writeback`.

fn mut_param(name: &str, type_name: &str) -> ParamDef {
    let mut p = param(name, TypeRef::Named(type_name.to_string()));
    p.is_ref = true;
    p.is_mut = true;
    p
}

fn unit_function(name: &str, params: Vec<ParamDef>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_crawler::{name}"),
        params,
        return_type: TypeRef::Unit,
        is_async: false,
        error_type: None,
        ..function(vec![])
    }
}

fn shim_context<'a>(
    type_paths: &'a HashMap<String, String>,
    unit_enum_names: &'a HashSet<&'a str>,
    tagged_enum_names: &'a HashSet<&'a str>,
    no_serde_names: &'a HashSet<&'a str>,
    handle_returned_types: &'a HashSet<String>,
    capsule_types: &'a HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    opaque_types: &'a ahash::AHashSet<String>,
) -> FunctionShimContext<'a> {
    FunctionShimContext {
        source_crate: "sample_crawler",
        type_paths,
        unit_enum_names,
        tagged_enum_names,
        no_serde_names,
        handle_returned_types,
        capsule_types,
        opaque_types,
    }
}

#[test]
fn mut_dto_param_returns_the_mirror_and_hands_back_the_mutation() {
    let f = unit_function("tag_record", vec![mut_param("record", "Record")]);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("pub fn tag_record(mut record: Record) -> Record {"),
        "writeback fn must return the wrapper type instead of (), got:\n{shim}"
    );
    assert!(
        shim.contains("sample_crawler::tag_record(&mut record.0)"),
        "writeback fn must still call the core fn with &mut on the inner field, got:\n{shim}"
    );
    assert!(
        shim.trim_end().ends_with("record\n}") || shim.trim_end().ends_with("record\n    }"),
        "writeback fn must hand the mutated wrapper back as its last expression, got:\n{shim}"
    );
    assert!(
        !shim.contains("pub fn tag_record(mut record: Record) {"),
        "the old void-returning declaration (silently drops the mutation) must be gone, got:\n{shim}"
    );
}

#[test]
fn immutable_borrow_dto_param_is_not_rewritten_as_writeback() {
    let mut p = param("record", TypeRef::Named("Record".to_string()));
    p.is_ref = true;
    let f = unit_function("read_record", vec![p]);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(
        !shim.contains("-> Record"),
        "an immutable-borrow param must not gain a wrapper-type return, got:\n{shim}"
    );
}

#[test]
fn owned_dto_param_is_byte_for_byte_unchanged() {
    let p = param("record", TypeRef::Named("Record".to_string()));
    let f = unit_function("consume_record", vec![p]);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert_eq!(
        shim, "pub fn consume_record(record: Record) {\n    sample_crawler::consume_record(record.0);\n}\n",
        "an owned (by-value) DTO param must render exactly as before, got:\n{shim}"
    );
}

#[test]
fn mut_opaque_param_is_not_treated_as_writeback() {
    let f = unit_function("bump_engine", vec![mut_param("engine", "Engine")]);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let mut opaque_types = ahash::AHashSet::default();
    opaque_types.insert("Engine".to_string());
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");
    assert!(
        !shim.contains("-> Engine"),
        "an opaque &mut param must not gain a writeback return, got:\n{shim}"
    );
    assert!(
        shim.contains("&mut engine.0"),
        "an opaque &mut param must keep mutating through its live handle, got:\n{shim}"
    );
}

#[test]
fn two_mut_dto_params_are_rejected_naming_the_function() {
    let f = unit_function(
        "tag_pair",
        vec![mut_param("first", "Record"), mut_param("second", "Record")],
    );
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let error = emit_function_shim(&f, &context).expect_err("two `&mut` DTO params must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("tag_pair"),
        "diagnostic must name the function: {message}"
    );
}

#[test]
fn mut_dto_param_plus_a_return_value_is_rejected_naming_the_function() {
    let mut f = unit_function("tag_and_count", vec![mut_param("record", "Record")]);
    f.return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::U32);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let error = emit_function_shim(&f, &context)
        .expect_err("a `&mut` DTO param on a function that also returns a value must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the function: {message}"
    );
}

/// Regression test for the alef CI `generated-output-gate` panic: swift-bridge-ir 0.1.59's
/// `BridgedType::to_alpha_numeric_underscore_name` (`bridged_type.rs:1986`) has a match arm for
/// every Rust integer primitive width except `u64`/`i64`; those two fall through to an
/// unconditional `todo!()`. Every `Result<Ok, String>` alef emits reaches that function (see
/// `result_ok_needs_json_bridge_with_handles`'s doc comment for why), so a fallible free function
/// declaring `Result<u64, String>` panicked `alef generate`'s own swift post-build, not just a
/// downstream consumer's build. Bridging the ok type through JSON avoids the panicking match arm
/// entirely.
#[test]
fn fallible_function_returning_u64_bridges_through_json_not_a_bare_u64() {
    let mut f = function(vec![]);
    f.return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::U64);
    f.is_async = false;

    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Result<String, String>"),
        "u64 Ok type must be bridged through JSON to dodge swift-bridge-ir's todo!() on \
         u64/i64, got:\n{shim}"
    );
    assert!(
        !shim.contains("Result<u64, String>"),
        "must never declare the panic-triggering Result<u64, String>, got:\n{shim}"
    );
    assert!(
        shim.contains("serde_json::to_string(&v)"),
        "the u64 value must be JSON-serialized to match the declared String Ok type, got:\n{shim}"
    );
}

/// Mirror of `fallible_function_returning_u64_bridges_through_json_not_a_bare_u64` for `i64`:
/// swift-bridge-ir's `to_alpha_numeric_underscore_name` match is missing both 64-bit integer
/// arms, not just the unsigned one.
#[test]
fn fallible_function_returning_i64_bridges_through_json_not_a_bare_i64() {
    let mut f = function(vec![]);
    f.return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::I64);
    f.is_async = false;

    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Result<String, String>"),
        "i64 Ok type must be bridged through JSON to dodge swift-bridge-ir's todo!() on \
         u64/i64, got:\n{shim}"
    );
    assert!(
        !shim.contains("Result<i64, String>"),
        "must never declare the panic-triggering Result<i64, String>, got:\n{shim}"
    );
}

/// The u64/i64 JSON-bridge above is scoped to the `Result` position only: a bare, infallible
/// `u64` return never reaches swift-bridge-ir's panicking path and must keep its native type,
/// not be forced through JSON needlessly.
#[test]
fn infallible_function_returning_u64_keeps_native_type() {
    let mut f = unit_function("count", vec![]);
    f.return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::U64);

    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("-> u64"),
        "an infallible u64 return must keep its native type, got:\n{shim}"
    );
    assert!(
        !shim.contains("serde_json::to_string"),
        "an infallible u64 return must not be JSON-bridged, got:\n{shim}"
    );
}

/// A Swift `Task.detached` closure calls this shim from a concurrency cooperative-pool thread,
/// whose stack is small and outside our control. `Runtime::block_on(future)` polls `future` on
/// the CALLING thread -- only a `Runtime::spawn`ed task runs on one of the runtime's own worker
/// threads (the ones actually sized via `thread_stack_size`). A deep extraction future polled
/// directly by `block_on` therefore still runs on the cooperative-pool thread's small stack and
/// can overflow it regardless of how large the runtime's worker stacks are configured. The async
/// shim must hand the real work to `.spawn(...)` and block the caller only on the resulting
/// `JoinHandle`, which is a shallow, constant-size wait.
#[test]
fn async_shim_spawns_the_future_and_blocks_only_on_the_join_handle() {
    let f = function(vec![]);
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains(&format!("{ALEF_TOKIO_RUNTIME_ACCESSOR}.spawn(async move {{")),
        "the deep work must be handed to `.spawn(async move {{ .. }})` so it runs on a \
         large-stack worker thread, not polled directly on the caller, got:\n{shim}"
    );
    assert!(
        !shim.contains(&format!("{ALEF_TOKIO_RUNTIME_ACCESSOR}.block_on(async {{")),
        "must not `.block_on` the raw future directly -- that runs the poll chain on the \
         calling (Swift cooperative-pool) thread's stack, got:\n{shim}"
    );
    assert!(
        shim.contains(&format!("{ALEF_TOKIO_RUNTIME_ACCESSOR}.block_on(__alef_task)")),
        "the calling thread must only block on the spawned task's `JoinHandle`, got:\n{shim}"
    );
    // Unwinding across the FFI boundary is undefined behavior. A panicked or cancelled task's
    // `JoinError` must become an ordinary `Err(String)`, never re-raised as a panic.
    assert!(
        shim.contains("unwrap_or_else(|__alef_join_error|") && shim.contains("Err(format!("),
        "a task panic or cancellation must surface as `Err(String)`, not unwind across the FFI \
         boundary, got:\n{shim}"
    );
    assert!(
        !shim.contains("panic!") && !shim.contains("resume_unwind"),
        "must not panic across the FFI boundary, got:\n{shim}"
    );
    assert!(
        shim.contains("-> Result<"),
        "every async shim must return a Result so a JoinError has somewhere to land, even when \
         the wrapped core call is itself infallible, got:\n{shim}"
    );
}

/// An async function with no `error_type` and no enum param would otherwise get a bare
/// (non-`Result`) return type -- but its shim still spawns a task whose `JoinHandle` can fail
/// independently of the wrapped call (panic, cancellation). Without a forced `Result` return,
/// that `JoinError` would have nowhere to go except a `panic!`, which is exactly what the FFI
/// boundary must never do. This is the case the previous test's default `error_type: Some(..)`
/// does not exercise.
#[test]
fn infallible_async_function_still_gets_forced_result_return_for_the_join_error() {
    let mut f = function(vec![]);
    f.error_type = None;
    let type_paths = HashMap::new();
    let empty_str = HashSet::new();
    let handle_returned_types = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &empty_str,
        &empty_str,
        &empty_str,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("-> Result<"),
        "an infallible async function must still be forced into a `Result`-shaped return so \
         the join-error path has somewhere to land, got:\n{shim}"
    );
    assert!(
        shim.contains("Ok("),
        "the success path must be wrapped in `Ok(..)`, got:\n{shim}"
    );
    assert!(
        !shim.contains("panic!") && !shim.contains("resume_unwind"),
        "must not panic across the FFI boundary, got:\n{shim}"
    );
}
