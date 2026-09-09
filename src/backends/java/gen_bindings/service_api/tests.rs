use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef, ServiceDef,
    TypeDef, TypeRef, WrapperConstructorCall,
};

/// Construct a minimal but realistic [`ApiSurface`] that exercises:
/// - A service with a constructor, one registration, and Run entrypoint
/// - One [`HandlerContractDef`] with wire request/response DTO names
fn make_fixture_surface() -> ApiSurface {
    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: "Create a new service owner.".to_owned(),
        receiver: None,
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let registration = RegistrationDef {
        method: "add_handler".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![ParamDef {
            name: "path".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: "Register a request handler.".to_owned(),
        variants: vec![
            crate::core::ir::RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "path".into(),
                    wrapper_type_path: "my_crate::Route".into(),
                    wrapper_type_name: "Route".into(),
                    constructor_method: "get".into(),
                    args: vec![],
                    options: Vec::new(),
                }),
                signature_params: vec![ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: Some("Register a GET handler.".to_owned()),
                style: Default::default(),
                ..Default::default()
            },
            crate::core::ir::RegistrationVariant {
                name: "post".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "path".into(),
                    wrapper_type_path: "my_crate::Route".into(),
                    wrapper_type_name: "Route".into(),
                    constructor_method: "post".into(),
                    args: vec![],
                    options: Vec::new(),
                }),
                signature_params: vec![ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: Some("Register a POST handler.".to_owned()),
                style: Default::default(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let run_ep = EntrypointDef {
        method: "run".to_owned(),
        kind: EntrypointKind::Run,
        is_async: true,
        params: vec![ParamDef {
            name: "addr".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        error_type: Some("ServiceError".to_owned()),
        doc: "Run the service.".to_owned(),
    };

    let service = ServiceDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![run_ep],
        doc: "A test service owner.".to_owned(),
        cfg: None,
    };

    let dispatch_method = MethodDef {
        name: "handle".to_owned(),
        params: vec![ParamDef {
            name: "request".to_owned(),
            ty: TypeRef::Named("RequestData".to_owned()),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("ResponseData".to_owned()),
        is_async: true,
        is_static: false,
        error_type: Some("HandlerError".to_owned()),
        doc: "Dispatch a request.".to_owned(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "my_crate::RequestHandler".to_owned(),
        dispatch: dispatch_method,
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_owned()),
        wire_response_type: Some("ResponseData".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Async trait for handling requests.".to_owned(),
    };

    ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            TypeDef {
                name: "RequestData".into(),
                has_serde: true,
                ..Default::default()
            },
            TypeDef {
                name: "ResponseData".into(),
                has_serde: true,
                ..Default::default()
            },
        ],
        services: vec![service],
        handler_contracts: vec![contract],
        ..ApiSurface::default()
    }
}

fn make_test_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "test-crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}

/// Assert that `java` emits `expected` as a whole line.
///
/// A `contains` on a fragment passes against output that merely embeds it — `contains("New")`
/// is satisfied by `public DownloadManager New(...)`. Matching a full line pins the indentation,
/// the modifiers and the argument order the fragment would let drift. ~keep
#[track_caller]
fn assert_emits_line(java: &str, expected: &str) {
    assert!(
        java.lines().any(|line| line == expected),
        "expected the generated service to emit exactly this line:\n{expected}\n\ngot:\n{java}"
    );
}

#[test]
fn java_class_uses_panama_ffm() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("import java.lang.foreign.*;"), "should import Panama FFM");
    assert!(java.contains("Linker.nativeLinker()"), "should use Linker");
    assert!(java.contains("downcallHandle"), "should use downcalls");
    assert!(java.contains("SymbolLookup"), "should lookup C symbols");
    assert!(java.contains("FunctionDescriptor"), "should build function descriptors");
    assert!(java.contains("MemorySegment"), "should use MemorySegment");
    assert!(java.contains("Arena"), "should use Arena for lifetime management");
}

#[test]
fn java_class_contains_service_class() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert_emits_line(&java, "public class TestService implements AutoCloseable {");
    // The owner handle is an `AlefHandle` (a `u64` registry key), not a pointer, so it is
    // carried as a `long`; commit 1e08f0ac7 "fix(java): align service wrappers with FFI ABI"
    // migrated it off `MemorySegment`. ~keep
    assert_emits_line(&java, "    private long ownerHandle;");
}

#[test]
fn java_class_constructor_uses_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("public TestService()"));
    assert!(
        java.contains("test_crate_test_service_new"),
        "constructor should bind to C symbol"
    );
    assert!(
        java.contains("LINKER.downcallHandle"),
        "constructor should use downcall"
    );
}

#[test]
fn java_class_contains_upcall_stub_for_handler() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("LINKER.upcallStub"),
        "registration should build upcall stub for handler"
    );
    assert!(java.contains("MethodHandle"), "should use MethodHandle to wrap handler");
}

#[test]
fn java_class_registration_binds_to_c_symbol() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("test_crate_test_service_register_add_handler"),
        "registration should bind to exact C FFI symbol"
    );
}

#[test]
fn java_class_entrypoint_uses_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("public void run(String addr)"));
    assert!(
        java.contains("test_crate_test_service_ep_run"),
        "entrypoint should bind to C symbol"
    );
    assert!(java.contains("LINKER.downcallHandle"), "entrypoint should use downcall");
}

#[test]
fn java_class_close_frees_via_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("@Override"));
    assert!(
        java.contains("test_crate_test_service_free"),
        "close should bind to C symbol"
    );
    assert!(java.contains("LINKER.downcallHandle"), "close should use downcall");
    assert_emits_line(&java, "    private synchronized OwnerHandleLease borrowOwnerHandle() {");
    // Detach, null-check and free all speak `long` since commit 1e08f0ac7 "fix(java): align
    // service wrappers with FFI ABI" replaced the `MemorySegment` owner handle with the FFI's
    // `AlefHandle` (`u64`). `invokeExact` is required: the descriptor declares JAVA_LONG. ~keep
    assert_emits_line(&java, "        long detached = takeOwnerHandleForClose();");
    assert_emits_line(&java, "            if (detached != 0) {");
    assert_emits_line(&java, "                freeHandle.invokeExact(detached);");
    assert_emits_line(&java, "                if (markServiceArenaClosed()) arena.close();");
    assert!(!java.contains("freeHandle.invokeExact(ownerHandle)"), "{java}");
    assert_emits_line(&java, "        while (activeOwnerBorrows != 0) {");
    assert_emits_line(&java, "        if (interrupted) Thread.currentThread().interrupt();");
    assert!(
        java.contains("public void close() {\n        synchronized (ownerMutationLock)"),
        "{java}"
    );
}

#[test]
fn java_class_leases_owner_for_every_service_downcall() {
    let surface = make_fixture_surface();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert_emits_line(&java, "        try (var ownerLease = borrowOwnerHandle();");
    assert_emits_line(&java, "                ownerLease.handle(),     // owner");
    assert_emits_line(&java, "            try (var ownerTransfer = takeOwnerHandle()) {");
    // `invokeWithArguments` (not `invoke`) since commit 7e44b62f9 "fix(java): harden owned
    // handle lifecycles": the argument list is built per entrypoint, so the call site cannot
    // name an exact `MethodType` at compile time. ~keep
    assert_emits_line(
        &java,
        "                int result = (int) epHandle.invokeWithArguments(ownerTransfer.handle()",
    );
    assert_emits_line(&java, "                ownerTransfer.commit();");
    assert!(java.find("ownerTransfer.handle()").unwrap() < java.find("ownerTransfer.commit()").unwrap());
    assert!(!java.contains("epHandle.invokeWithArguments(ownerHandle"), "{java}");
    assert!(
        !java.contains("applyHandle.invoke"),
        "unsupported configurators must not be emitted:\n{java}"
    );
    assert!(
        !java.contains("public void config(String host, int port)"),
        "the hardcoded ServerConfig method binds C symbols no surface declares:\n{java}"
    );
}

#[test]
fn java_finalize_returns_opaque_handle_and_consumes_owner() {
    let mut surface = make_fixture_surface();
    surface.types.push(crate::core::ir::TypeDef {
        name: "Router".into(),
        is_opaque: true,
        ..Default::default()
    });
    let entrypoint = &mut surface.services[0].entrypoints[0];
    entrypoint.kind = EntrypointKind::Finalize;
    entrypoint.return_type = TypeRef::Named("Router".into());

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    // The FFI returns this entrypoint as `AlefHandle` (`u64`), so the wrapper carries it as a
    // `long`. It cannot yet be rewrapped as `new Router(...)`: the generated opaque class takes
    // `Router(MemorySegment)`, and an `AlefHandle` is a registry key, not an address — feeding
    // one to `MemorySegment.ofAddress` would fabricate a pointer. ~keep
    assert_emits_line(&java, "    public long run(String addr) {");
    assert_emits_line(
        &java,
        "                long result = (long) epHandle.invokeWithArguments(ownerTransfer.handle()",
    );
    assert_emits_line(&java, "            try (var ownerTransfer = takeOwnerHandle()) {");
    assert_emits_line(&java, "                ownerTransfer.commit();");
    assert_emits_line(
        &java,
        "                    throw new IllegalStateException(\"Service finalizer returned null\");",
    );
    assert_emits_line(&java, "                return result;");
}

#[test]
fn generated_service_owner_gate_compiles_and_blocks_close_until_release() {
    if crate::test_support::spawn_from_stable_dir("javac")
        .arg("-version")
        .output()
        .is_err()
    {
        return;
    }
    let surface = make_fixture_surface();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());
    let directory = tempfile::tempdir().expect("temporary service directory");
    let package_directory = directory.path().join("com/example");
    std::fs::create_dir_all(&package_directory).expect("service package directory");
    std::fs::write(package_directory.join("TestService.java"), java).expect("generated service");
    std::fs::write(
        package_directory.join("NativeLib.java"),
        "package com.example; final class NativeLib {}\n",
    )
    .expect("NativeLib stub");
    std::fs::write(
        package_directory.join("Callable.java"),
        "package com.example; interface Callable { String handle(String request); }\n",
    )
    .expect("Callable stub");
    std::fs::write(
        package_directory.join("ServiceGateMain.java"),
        include_str!("../../../../../tests/fixtures/java_service_gate_main.java"),
    )
    .expect("service gate runtime");

    let compile = std::process::Command::new("javac")
        .args([
            "com/example/NativeLib.java",
            "com/example/Callable.java",
            "com/example/TestService.java",
            "com/example/ServiceGateMain.java",
        ])
        .current_dir(directory.path())
        .output()
        .expect("javac service gate");
    assert!(
        compile.status.success(),
        "generated service must compile:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "com.example.ServiceGateMain"])
        .current_dir(directory.path())
        .output()
        .expect("run service gate");
    assert!(
        run.status.success(),
        "service owner gate runtime must pass:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn java_class_no_native_method_declarations() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        !java.contains("public native ")
            && !java.contains("private native ")
            && !java.contains("protected native ")
            && !java.contains("static native "),
        "should not contain JNI native method declarations:\n{java}"
    );
    assert!(
        !java.contains("System.loadLibrary"),
        "should not load library (Panama manages it)"
    );
    assert!(!java.contains("Java_"), "should not contain Java_ JNI symbols");
}

#[test]
fn callable_interface_is_functional() {
    let iface = gen_callable_interface("com.example");

    assert!(iface.contains("@FunctionalInterface"));
    assert!(iface.contains("public interface Callable"));
    assert!(iface.contains("String handle(String request)"));
}

#[test]
fn generate_returns_service_and_callable() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    let config = make_test_config();

    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.len() >= 2, "expected at least service class + Callable interface");

    let has_service_class = files
        .iter()
        .any(|f| f.path.to_string_lossy().contains("TestService.java"));
    let has_callable = files.iter().any(|f| f.path.to_string_lossy().contains("Callable.java"));

    assert!(has_service_class, "expected TestService.java");
    assert!(has_callable, "expected Callable.java");
}

#[test]
fn generate_returns_empty_for_no_services() {
    let surface = ApiSurface::default();
    let config = make_test_config();

    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

#[test]
fn generate_rejects_unsupported_service_abi_shapes() {
    let config = make_test_config();

    let mut optional = make_fixture_surface();
    optional.services[0].registrations[0].variants.clear();
    optional.services[0].registrations[0].metadata_params[0].optional = true;
    let error = generate(&optional, &config).expect_err("optional service metadata must be rejected");
    assert!(error.to_string().contains("TestService.add_handler.path"), "{error:#}");

    let mut bytes = make_fixture_surface();
    bytes.services[0].registrations[0].variants.clear();
    bytes.services[0].entrypoints[0].params[0].ty = TypeRef::Bytes;
    let error = generate(&bytes, &config).expect_err("bytes without a length carrier must be rejected");
    assert!(error.to_string().contains("TestService.run.addr"), "{error:#}");

    let mut variants = make_fixture_surface();
    variants.services[0].registrations[0].variants[0].wrapper_call = None;
    let error = generate(&variants, &config).expect_err("variants without FFI wrapper calls must be rejected");
    assert!(error.to_string().contains("TestService.add_handler.get"), "{error:#}");
}

/// A Finalize entrypoint returning a type the surface does not wrap crosses the C ABI as an
/// `i32` status, so its value is unreachable from Java. That is worth a warning, not a refusal:
/// the C export exists, the status is meaningful, and the csharp backend already ships this exact
/// shape (`public int into_router()`).
///
/// This test asserted the refusal. Under it, one lossy entrypoint cost java its entire binding --
/// spikard's `App.into_router` returns a `String`, so java emitted no service class at all and
/// every generated route registration in the e2e suite answered 404. ~keep
#[test]
fn a_lossy_finalize_return_warns_rather_than_refusing_the_whole_service() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].entrypoints[0].kind = EntrypointKind::Finalize;
    surface.services[0].entrypoints[0].return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::I32);

    let files = generate(&surface, &make_test_config()).expect("a lossy finalize must not refuse the service");

    assert!(
        files.iter().any(|file| file.path.ends_with("TestService.java")),
        "the service class must still be emitted, got: {:?}",
        files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn generated_service_uses_native_width_primitive_carriers() {
    // Every primitive metadata shape, with the `ValueLayout` and argument expression the C ABI
    // requires. `backends::ffi`'s `typeref_to_rust_ffi_type` declares each parameter at its
    // native Rust width (`i32` stays `i32`), so widening them all to a shared `long` carrier
    // would be an ABI mismatch — and `invokeWithArguments` only widens, so a boxed `Long`
    // against a `JAVA_INT` layout fails at call time rather than compile time. ~keep
    let carriers: [(&str, PrimitiveType, &str, &str); 13] = [
        (
            "flag",
            PrimitiveType::Bool,
            "ValueLayout.JAVA_BYTE",
            "(byte) (flag ? 1 : 0)",
        ),
        ("u8_value", PrimitiveType::U8, "ValueLayout.JAVA_BYTE", "u8Value"),
        ("u16_value", PrimitiveType::U16, "ValueLayout.JAVA_SHORT", "u16Value"),
        ("u32_value", PrimitiveType::U32, "ValueLayout.JAVA_INT", "u32Value"),
        ("u64_value", PrimitiveType::U64, "ValueLayout.JAVA_LONG", "u64Value"),
        ("i8_value", PrimitiveType::I8, "ValueLayout.JAVA_BYTE", "i8Value"),
        ("i16_value", PrimitiveType::I16, "ValueLayout.JAVA_SHORT", "i16Value"),
        ("i32_value", PrimitiveType::I32, "ValueLayout.JAVA_INT", "i32Value"),
        ("i64_value", PrimitiveType::I64, "ValueLayout.JAVA_LONG", "i64Value"),
        (
            "usize_value",
            PrimitiveType::Usize,
            "ValueLayout.JAVA_LONG",
            "usizeValue",
        ),
        (
            "isize_value",
            PrimitiveType::Isize,
            "ValueLayout.JAVA_LONG",
            "isizeValue",
        ),
        ("f32_value", PrimitiveType::F32, "ValueLayout.JAVA_FLOAT", "f32Value"),
        ("f64_value", PrimitiveType::F64, "ValueLayout.JAVA_DOUBLE", "f64Value"),
    ];

    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].registrations[0].metadata_params = carriers
        .iter()
        .map(|(name, primitive, _, _)| ParamDef {
            name: (*name).into(),
            ty: TypeRef::Primitive(primitive.clone()),
            ..Default::default()
        })
        .collect();

    let files = generate(&surface, &make_test_config()).expect("supported primitive service carriers");
    let java = &files
        .iter()
        .find(|file| file.path.ends_with("TestService.java"))
        .expect("service class")
        .content;

    for (name, _, layout, arg) in &carriers {
        let camel = name.to_lower_camel_case();
        assert_emits_line(java, &format!("                , {layout}    // {camel} param"));
        assert_emits_line(java, &format!("                , {arg}    // metadata"));
    }
    assert!(!java.contains("(long) i32Value"), "{java}");
    assert!(!java.contains("(long) ((flag ? 1 : 0))"), "{java}");
}

/// A `Named` metadata parameter naming a type on this surface is carried, not refused: the C
/// export declares it as an `AlefHandle` and this backend already renders `ValueLayout.JAVA_LONG`
/// with `<param>.handle().address()` for it.
///
/// The guard this replaces refused every one of them with "unsupported until the generated C
/// header declares a carrier" -- a carrier the header had declared all along. It never ran,
/// because java's exclusion pass dropped the service before validation could reach it, so the
/// claim went untested against a real surface for as long as it existed. ~keep
#[test]
fn a_named_metadata_param_on_the_surface_is_carried_as_a_handle() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].registrations[0].metadata_params[0].ty = TypeRef::Named("RequestData".into());

    let files = generate(&surface, &make_test_config()).expect("a surface-defined named param must be carried");

    assert!(
        files.iter().any(|file| file.path.ends_with("TestService.java")),
        "the service class must still be emitted"
    );
}

/// Negative control for the above: a `Named` type the surface does not define has no handle
/// carrier at all, and `java_type_for_metadata` would degrade it to `Object`. That is still a
/// refusal -- without this, "accept every Named param" would pass the test above while emitting a
/// signature naming a class that does not exist.
#[test]
fn a_named_metadata_param_absent_from_the_surface_is_still_rejected() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].registrations[0].metadata_params[0].ty = TypeRef::Named("NotOnThisSurface".into());

    let error = generate(&surface, &make_test_config()).expect_err("an undefined named param must be rejected");

    assert!(
        error.to_string().contains("is not defined on this surface"),
        "{error:#}"
    );
}

#[test]
fn java_backend_service_generation_suppresses_lifetime_bound_types() {
    use crate::core::backend::Backend;

    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "BorrowedConfig".into(),
        has_lifetime_params: true,
        ..Default::default()
    });
    surface.services[0].entrypoints[0].params[0].ty = TypeRef::Named("BorrowedConfig".into());

    let files = crate::backends::java::JavaBackend
        .generate_service_api(&surface, &make_test_config())
        .expect("filtered Java service generation");

    assert!(files.is_empty(), "lifetime-bound service signatures must be suppressed");
}

/// Java marshals neither direction of the callback: the C export declares it as
/// `char *(*)(void*, const char*)`, so the generated Java hands the JSON straight through as a
/// `String` and never names the wire type in any signature it emits. Producing that JSON is the
/// FFI crate's job.
///
/// The assertion this replaces read `!typ.has_serde`, and `has_serde` records *derives*. It is
/// false for a type with hand-written `impl Serialize`/`impl Deserialize`, and false again for a
/// type alef resolved at a re-export site rather than at its definition. spikard's `RequestData`
/// is both at once, and the FFI crate serialises it perfectly well. ~keep
#[test]
fn a_callback_wire_type_without_serde_derives_is_accepted() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface
        .types
        .iter_mut()
        .find(|typ| typ.name == "RequestData")
        .unwrap()
        .has_serde = false;

    let files =
        generate(&surface, &make_test_config()).expect("a wire type without serde derives must not refuse the service");

    assert!(
        files.iter().any(|file| file.path.ends_with("TestService.java")),
        "the service class must still be emitted"
    );
}

/// Negative control: a contract that names no wire type at all is still a refusal. That one is
/// about alef's own configuration being coherent, not about what java can marshal, so relaxing
/// the serde assertion must not take it with it.
#[test]
fn a_callback_contract_naming_no_wire_type_is_still_rejected() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.handler_contracts[0].wire_request_type = None;

    let error = generate(&surface, &make_test_config()).expect_err("a missing wire type must be rejected");

    assert!(
        error.to_string().contains("the callback request type is missing"),
        "{error:#}"
    );
}

#[test]
fn java_class_passes_all_metadata_params() {
    let mut surface = make_fixture_surface();
    let reg = &mut surface.services[0].registrations[0];

    reg.metadata_params.push(ParamDef {
        name: "method".to_owned(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        ..ParamDef::default()
    });
    reg.metadata_params.push(ParamDef {
        name: "priority".to_owned(),
        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
        optional: false,
        default: None,
        ..ParamDef::default()
    });

    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("public int registerTestServiceAddHandler(Callable handler, String path"),
        "registration method must include all metadata parameters"
    );

    assert!(
        java.contains("ValueLayout.ADDRESS") || java.contains("ValueLayout.JAVA_INT"),
        "registration should build FunctionDescriptor with metadata param layouts"
    );
}

#[test]
fn java_class_does_not_borrow_record_metadata_as_an_opaque_handle() {
    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "RequestOptions".to_owned(),
        rust_path: "test_crate::RequestOptions".to_owned(),
        is_opaque: false,
        ..TypeDef::default()
    });
    surface.services[0].registrations[0].metadata_params.push(ParamDef {
        name: "options".to_owned(),
        ty: TypeRef::Named("RequestOptions".to_owned()),
        ..ParamDef::default()
    });

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert!(!java.contains("RequestOptions.HandleLease"), "{java}");
    assert!(!java.contains("options.borrowHandle()"), "{java}");
}

#[test]
fn java_class_marshals_service_metadata_to_ffi_carriers() {
    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "RequestOptions".to_owned(),
        rust_path: "test_crate::RequestOptions".to_owned(),
        ..TypeDef::default()
    });
    surface.services[0].registrations[0].metadata_params.extend([
        ParamDef {
            name: "options".to_owned(),
            ty: TypeRef::Named("RequestOptions".to_owned()),
            ..ParamDef::default()
        },
        ParamDef {
            name: "priority".to_owned(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            ..ParamDef::default()
        },
    ]);

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    // Metadata strings are copied into an owned Rust `String` by the FFI before it returns, so
    // they belong in the per-call confined arena; the class-scoped `Arena.ofShared()` exists for
    // upcall stubs, which must outlive the call, and allocating there leaks one buffer a call. ~keep
    assert_emits_line(&java, "            var cpath = callArena.allocateFrom(path);");
    assert_emits_line(&java, "                , ValueLayout.ADDRESS    // path param");
    assert_emits_line(&java, "                , ValueLayout.JAVA_INT    // priority param");
    assert_emits_line(&java, "                , cpath    // metadata");
    assert_emits_line(&java, "                , priority    // metadata");
    assert_emits_line(&java, "                , cpath");
    assert!(java.contains("varHandle.invokeWithArguments("), "{java}");
    assert!(!java.contains("invokeExact(args)"), "{java}");
    // A named metadata parameter crosses as an `AlefHandle`, which the descriptor carries as
    // `JAVA_LONG` and the call site passes as `options.handle().address()`. The renderer must not
    // invent a `*_from_json` round trip for it: that symbol is not in the generated header, and
    // the handle is already the carrier. ~keep
    assert!(!java.contains("TEST_CRATE_REQUEST_OPTIONS_FROM_JSON"), "{java}");
    assert!(!java.contains("nativeResources.register(cOptions"), "{java}");
    assert_emits_line(&java, "                , ValueLayout.JAVA_LONG    // options param");
    generate(&surface, &make_test_config()).expect("a surface-defined named metadata param is carried");
}

#[test]
fn java_class_emits_registration_variants() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    // Variant shortcuts take their metadata first and the handler last, mirroring the Rust
    // builder they shadow (`app.get("/path", handler)`). Commit ab729d0e3 "fix(java): restore
    // service registration contracts" put the order back after it had been flipped to
    // handler-first; the `register*` methods keep handler-first because they are not shortcuts. ~keep
    assert_emits_line(&java, "    public int get(String path, Callable handler) {");
    assert_emits_line(&java, "    public int post(String path, Callable handler) {");

    assert!(
        java.contains("test_crate_test_service_get"),
        "should bind get variant to correct C symbol"
    );
    assert!(
        java.contains("test_crate_test_service_post"),
        "should bind post variant to correct C symbol"
    );

    assert!(
        java.contains("LINKER.downcallHandle"),
        "variant methods should use Panama downcalls"
    );
    assert!(
        java.contains("LINKER.upcallStub"),
        "variant methods should create upcall stubs"
    );
    assert!(
        java.contains("FunctionDescriptor.of"),
        "variant methods should build function descriptors"
    );
}

/// Assert that `file` carries an alef marker, that the bytes the writer would put on disk
/// still carry it, and that the injected `alef:hash:` line re-verifies the way `alef verify`
/// derives it. A `.java` file that fails the marker check reaches `finalize_hashes` but
/// carries no marker. A `.java` file that fails this gets neither provenance nor any
/// future regeneration, silently. ~keep
fn assert_pipeline_stamps(file: &GeneratedFile) {
    use crate::core::hash;

    let path = file.path.display().to_string();
    assert!(
        file.carries_alef_marker(),
        "{path}: emitted without an alef marker and without `generated_header`, so the \
         path never reaches `finalize_hashes` and the write guard refuses to rewrite it"
    );

    let on_disk = if hash::content_has_alef_marker(&file.content) {
        file.content.clone()
    } else {
        format!("{}\n{}", hash::header(hash::CommentStyle::DoubleSlash), file.content)
    };
    assert!(
        hash::content_has_alef_marker(&on_disk),
        "{path}: the bytes the writer puts on disk must carry the marker `finalize_hashes` \
         searches for, got:\n{on_disk}"
    );

    let body = hash::strip_hash_line(&on_disk);
    let stamped = hash::inject_hash_line(&body, &hash::compute_file_hash(&body));
    assert_eq!(
        hash::extract_hash(&stamped),
        Some(hash::compute_file_hash(&hash::strip_hash_line(&stamped))),
        "{path}: the injected alef:hash: line must re-verify the way `alef verify` derives it"
    );
}

#[test]
fn every_emitted_java_service_file_carries_a_hash_line_after_finalize() {
    let surface = make_fixture_surface();
    let config = make_test_config();

    let files = generate(&surface, &config).expect("java service api generation");

    let named = |name: &str| {
        files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with(name))
            .unwrap_or_else(|| {
                panic!(
                    "{name} must be emitted; got {:?}",
                    files.iter().map(|file| &file.path).collect::<Vec<_>>()
                )
            })
    };

    // Positive control: assert each file actually holds its generated payload, so the
    // stamping assertions below cannot pass over empty or missing output. ~keep
    assert!(
        named("TestService.java")
            .content
            .contains("public class TestService implements AutoCloseable"),
        "TestService.java must hold the real service wrapper, got:\n{}",
        named("TestService.java").content
    );
    assert!(
        named("Callable.java").content.contains("public interface Callable"),
        "Callable.java must hold the handler interface, got:\n{}",
        named("Callable.java").content
    );

    for file in &files {
        assert_pipeline_stamps(file);
    }
}
