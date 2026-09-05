//! Dart-specific e2e generator tests.

use super::stubs::emit_test_backend;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
use crate::e2e::fixture::Fixture;

fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        ..Default::default()
    }
}

fn make_method(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Verify that no sample_core-domain names leak into the generated output when
/// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
#[test]
fn dart_stub_contains_no_sample_crate_domain_names() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("SampleCrate"),
        "must not contain literal 'SampleCrate', got:\n{output}"
    );
    assert!(
        !output.contains("sample_crate::"),
        "must not contain 'sample_crate::', got:\n{output}"
    );
    assert!(
        !output.contains("SampleCrateBridge"),
        "must not contain 'SampleCrateBridge', got:\n{output}"
    );
    assert!(
        output.contains("TestStubMyTestFixture"),
        "class name must be derived from fixture id, got:\n{output}"
    );
    assert!(
        output.contains("extends TestTrait"),
        "class must extend the configured trait class, got:\n{output}"
    );
    assert!(
        output.contains("doWork"),
        "required method must be emitted, got:\n{output}"
    );
}

fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
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

fn make_method_with_params(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![
            make_param("content", TypeRef::Bytes),
            make_param("mime_type", TypeRef::String),
        ],
        return_type: TypeRef::Named("SampleResult".to_string()),
        is_async: true,
        is_static: false,
        error_type: Some("anyhow::Error".to_string()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// Verify params use concrete Dart types (not `dynamic`) and no @override annotation.
#[test]
fn dart_stub_uses_typed_params_not_dynamic() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method_with_params("extract", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("dynamic content"),
        "param must not use `dynamic`, got:\n{output}"
    );
    assert!(
        output.contains("Uint8List content"),
        "bytes param must map to Uint8List, got:\n{output}"
    );
    assert!(
        output.contains("String mimeType"),
        "string param must map to String, got:\n{output}"
    );
    assert!(
        output.contains("Future<SampleResult>"),
        "return type must be concrete not dynamic, got:\n{output}"
    );
    assert!(
        !output.contains("@override"),
        "local class members must not use @override annotation, got:\n{output}"
    );
}

/// Verify that `fixture.input["name"]` is used as the plugin name when present.
#[test]
fn dart_stub_uses_fixture_input_name_for_plugin_name() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let mut fixture = make_fixture("my_fixture_id");
    fixture.input = serde_json::json!({ "name": "my-backend-name" });

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("'my-backend-name'"),
        "plugin name must come from fixture.input.name, got:\n{output}"
    );
    assert!(
        !output.contains("my_fixture_id"),
        "fixture id must not appear as plugin name when input.name is set, got:\n{output}"
    );
}

/// Verify that _setEnv helper forces overwrite=1 and checks return code.
/// Regression test for the bug where setenv(..., 0) silently no-ops when the
/// env var is already set, causing SAMPLE_ALLOW_PRIVATE_NETWORK to be
/// invisible to Rust FFI dylib in dart e2e tests.
#[test]
fn dart_emit_setenv_forces_overwrite_and_checks_return_code() {
    use crate::e2e::config::E2eConfig;
    use std::collections::BTreeMap;

    // Create a minimal E2eConfig with an env var to trigger _setEnv emission.
    let mut env = BTreeMap::new();
    env.insert("SAMPLE_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());

    let e2e_config = E2eConfig {
        env,
        ..Default::default()
    };

    // Build a minimal test file just to check the _setEnv helper.
    // We'll use a dummy fixture and configuration.
    let fixture = make_fixture("test_fixture");
    let _bridge = make_trait_bridge("TestTrait");
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs = [];
    let enums = [];
    let adapters = [];
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &adapters,
        &config,
        &type_defs,
        &enums,
        &[],
        &[],
    );

    // Verify that the generated setenv call uses overwrite=1 (third argument).
    assert!(
        output.contains("setenv(keyPtr, valuePtr, 1)"),
        "setenv must use overwrite=1, got:\n{output}"
    );

    // Verify that the old buggy pattern is NOT in the output.
    assert!(
        !output.contains("setenv(keyPtr, valuePtr, 0)"),
        "setenv must NOT use overwrite=0, got:\n{output}"
    );

    // Verify that return code is captured and checked.
    assert!(
        output.contains("final result = setenv(keyPtr, valuePtr, 1)"),
        "setenv result must be captured, got:\n{output}"
    );

    assert!(
        output.contains("if (result != 0)"),
        "return code must be checked with 'if (result != 0)', got:\n{output}"
    );

    assert!(
        output.contains("throw StateError"),
        "must throw StateError on non-zero return code, got:\n{output}"
    );
}

/// An `error` assertion with a declared `value` must produce a `throwsA`
/// predicate matcher that checks both the caught error's `toString()` and its
/// `runtimeType.toString()`, since fixture authors use either a message-only
/// field name or a type-name prefix.
#[test]
fn dart_error_assertion_with_declared_value_checks_message_and_type() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("ThingNotFound")),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(
        output.contains(
            "throwsA(predicate((e) => e.toString().contains('ThingNotFound') || e.runtimeType.toString().contains('ThingNotFound')))"
        ),
        "expected a disjunctive message-or-type predicate matcher against the declared value, got:\n{output}"
    );
    assert!(
        !output.contains("throwsA(anything)"),
        "declared value must replace the anything-matcher, got:\n{output}"
    );
}

/// With no declared `value` on the `error` assertion, output must be
/// byte-identical to the pre-existing `throwsA(anything)` behavior.
#[test]
fn dart_error_assertion_without_declared_value_is_byte_identical() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(output.contains("throwsA(anything)"));
    assert!(!output.contains("predicate((e)"));
}

/// Declared error values containing Dart string-interpolation and escape
/// characters (`'`, `\`, `$`) must be escaped via the shared `escape_dart`
/// helper, not hand-rolled, so the emitted literal stays a valid single-quoted
/// Dart string.
#[test]
fn dart_error_assertion_escapes_declared_value_for_dart_string_literal() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("bad 'field' \\ $value")),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    let expected_escaped = super::values::escape_dart("bad 'field' \\ $value");
    let expected_snippet = format!("e.toString().contains('{expected_escaped}')");
    assert!(
        output.contains(&expected_snippet),
        "expected escaped literal snippet `{expected_snippet}` in:\n{output}"
    );
}

#[test]
fn dart_test_file_emits_wrapper_for_call_config_trait_argument() {
    let fixture = make_fixture("register_backend");
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "registerBackend".into();
    e2e_config.call.args.push(crate::e2e::config::ArgMapping {
        name: "backend".into(),
        field: "input.backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("TestTrait".into()),
    });
    let mut config = crate::core::config::ResolvedCrateConfig::default();
    config.trait_bridges.push(make_trait_bridge("TestTrait"));
    let type_defs = [crate::core::ir::TypeDef {
        name: "TestTrait".into(),
        methods: vec![make_method("doWork", true)],
        ..Default::default()
    }];
    let output = super::test_file::render_test_file(
        "plugins",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &crate::e2e::field_access::DartFirstClassMap::default(),
        &[],
        &config,
        &type_defs,
        &[],
        &[],
        &[],
    );
    assert!(output.contains("Future<TestTraitDartImpl> _createTestStubRegisterBackendWrapper()"));
    assert!(output.contains("await _createTestStubRegisterBackendWrapper()"));
    assert_eq!(
        output.matches("import 'package:samplecli/samplecli.dart'").count(),
        1,
        "the broad package import already exposes TestTrait; a second show-import is redundant:\n{output}"
    );
    assert!(
        !output.contains("package:samplecli/samplecli.dart' show TestTrait"),
        "a redundant show-import triggers duplicate_import in dart analyze:\n{output}"
    );
}

#[test]
fn dart_trait_stub_wrapper_compiles() {
    if crate::test_support::spawn_from_stable_dir("dart")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let method = make_method("doWork", true);
    let emission = emit_test_backend(
        &make_trait_bridge("TestTrait"),
        &[&method],
        &make_fixture("register_backend"),
        &[],
    );
    let source = format!(
        "abstract class TestTrait {{ Future<bool> doWork(); }}\nclass TestTraitDartImpl {{}}\nFuture<TestTraitDartImpl> createTestTraitDartImpl({{required String pluginName, required String pluginVersion, required Future<bool> Function() doWork}}) async => TestTraitDartImpl();\n{}\nFuture<void> main() async {{ await _createTestStubRegisterBackendWrapper(); }}\n",
        emission.setup_block
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("stub.dart");
    std::fs::write(&source_path, &source).expect("write Dart source");
    // Pin the child's working directory. Other tests in this binary mutate the
    // process-global cwd via `set_current_dir` into tempdirs that are then dropped, so
    // an inherited cwd can already be deleted by the time this runs -- the Dart VM then
    // fails startup with "Error determining current directory" rather than any analysis
    // result. ~keep
    let output = std::process::Command::new("dart")
        .args(["analyze", "--fatal-infos"])
        .arg(&source_path)
        .current_dir(directory.path())
        .output()
        .expect("run Dart analyzer");
    assert!(
        output.status.success(),
        "dart analyze failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}\n--- source ---\n{source}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Dart's error path renders `expectLater(..., throwsA(..))` and returns, so every other
/// assertion on the fixture used to leave no trace in the generated test at all.
#[test]
fn dart_equals_on_an_error_field_is_named_instead_of_dropped() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("rate_limited");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("ThingNotFound")),
        ..Default::default()
    });
    fixture.assertions.push(Assertion {
        assertion_type: "equals".into(),
        field: Some("error.status_code".into()),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let _ = crate::e2e::codegen::take_skip_records();
    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    // Positive first: the error block really rendered.
    assert!(
        output.contains("throwsA(predicate("),
        "the error block must render: {output}"
    );
    assert!(
        output.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "{output}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "dart");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated file marker-free.
#[test]
fn dart_a_lone_error_assertion_renders_no_marker() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(output.contains("throwsA("), "the error block must render: {output}");
    assert!(!output.contains("has no accessor for error field"), "{output}");
}

// --- typed-argument lowering (alef #227) -----------------------------------------------------

/// Render one fixture through the real dart test-file path with an `args` entry that has the
/// default `arg_type` (`"string"`) -- the shape that used to emit a quoted literal regardless of
/// what the parameter it fills is declared as.
///
/// `functions` is the only thing that varies between the two states below: an empty slice with an
/// empty `type_defs` is exactly `TargetParams::IrAbsent`, the state every unconverted caller is in.
fn render_dart_enum_arg_fixture(
    functions: &[crate::core::ir::FunctionDef],
    enums: &[crate::core::ir::EnumDef],
) -> String {
    let mut fixture = make_fixture("upload");
    fixture.input = serde_json::json!({ "purpose": "fine-tune" });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "upload_file".into();
    e2e_config.call.args = vec![crate::e2e::config::ArgMapping {
        name: "purpose".to_string(),
        field: "input.purpose".to_string(),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];

    super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &crate::e2e::field_access::DartFirstClassMap::default(),
        &[],
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
        enums,
        functions,
        &[],
    )
}

fn file_purpose_enum() -> crate::core::ir::EnumDef {
    crate::core::ir::EnumDef {
        name: "FilePurpose".to_string(),
        has_serde: true,
        serde_rename_all: Some("kebab-case".to_string()),
        variants: vec![crate::core::ir::EnumVariant {
            name: "FineTune".to_string(),
            ..crate::core::ir::EnumVariant::default()
        }],
        ..crate::core::ir::EnumDef::default()
    }
}

fn upload_file_taking(type_name: &str) -> crate::core::ir::FunctionDef {
    crate::core::ir::FunctionDef {
        name: "upload_file".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "purpose".to_string(),
            ty: TypeRef::Named(type_name.to_string()),
            ..crate::core::ir::ParamDef::default()
        }],
        return_type: TypeRef::Named("UploadedFile".to_string()),
        ..crate::core::ir::FunctionDef::default()
    }
}

/// The defect: a fixture string bound for an enum-typed parameter stayed a *string literal*, which
/// the Dart analyzer rejects against a generated `enum`. The variant here is
/// `rename_all = "kebab-case"`, the case a naive camel-casing of the wire value would get wrong. ~keep
#[test]
fn a_string_value_for_an_ir_enum_parameter_names_the_generated_dart_variant() {
    let output = render_dart_enum_arg_fixture(&[upload_file_taking("FilePurpose")], &[file_purpose_enum()]);

    assert!(
        output.contains("FilePurpose.fineTune"),
        "expected the generated Dart enum variant, got:\n{output}"
    );
    assert!(
        !output.contains("'fine-tune'"),
        "the string literal must be replaced, not accompanied, got:\n{output}"
    );
}

/// The other half of the three-state trade. Identical fixture, arg and enum; only the core IR is
/// withheld. The pre-seam lowering must survive verbatim, or every IR-less caller regresses
/// silently -- which is the whole reason `TargetParams` has three states and not two. ~keep
#[test]
fn the_same_string_value_still_renders_as_a_dart_literal_when_the_ir_is_absent() {
    let output = render_dart_enum_arg_fixture(&[], &[file_purpose_enum()]);

    assert!(
        output.contains("'fine-tune'"),
        "the IR-less path must keep today's string literal, got:\n{output}"
    );
    assert!(
        !output.contains("FilePurpose.fineTune"),
        "an absent IR licenses no type claim, got:\n{output}"
    );
}

/// A wire value naming no variant is very often a deliberately invalid value driving the binding's
/// own validation, so it keeps its literal rather than gaining an invented variant. ~keep
#[test]
fn an_unmatched_dart_enum_wire_value_keeps_its_string_literal() {
    let mut unmatched = file_purpose_enum();
    unmatched.variants[0].name = "Assistants".to_string();
    let output = render_dart_enum_arg_fixture(&[upload_file_taking("FilePurpose")], &[unmatched]);

    assert!(
        output.contains("'fine-tune'"),
        "an unmatched wire value must keep its literal, got:\n{output}"
    );
}

/// A declared type that is not an enum at all is not a licence to invent a variant reference: it
/// may be a newtype the Dart binding flattens to a `String`. ~keep
#[test]
fn a_declared_type_that_is_not_an_ir_enum_keeps_the_existing_dart_lowering() {
    let output = render_dart_enum_arg_fixture(&[upload_file_taking("PromptText")], &[file_purpose_enum()]);

    assert!(
        output.contains("'fine-tune'"),
        "a non-enum declared type must keep today's literal, got:\n{output}"
    );
}

/// Regression for the standalone-mock-server spawn re-deriving
/// `Directory.current.uri.resolve('../rust/Cargo.toml')`, which only resolves correctly when
/// `Directory.current` is `e2e/dart/`. The generated test's own `Process.run` invocation
/// (`--manifest-path`, cargo's own error) reproduced as `Bad state: mock-server build failed:
/// error: manifest path ... does not exist` whenever the process was started from any other
/// cwd -- six suites failing in `setUpAll` with zero fixture assertions run. The fix routes
/// the standalone-mock-server branch through `startMockServer()` in the shared, alef-generated
/// `e2e_helpers.dart` (`DartE2eCodegen::generate` / `project::render_e2e_helpers`), whose
/// `_findRepoRoot()` walks up from `Directory.current` to a stable `Cargo.toml` +
/// `test_documents/` marker instead of resolving a fixed relative path against it.
#[test]
fn dart_standalone_mock_server_spawn_delegates_to_the_shared_helper() {
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    let mut fixture = make_fixture("http_fixture");
    fixture.http = Some(HttpFixture {
        handler: HttpHandler {
            route: "/user".to_string(),
            method: "GET".to_string(),
            body_schema: None,
            parameters: Default::default(),
            middleware: None,
        },
        request: HttpRequest {
            method: "GET".to_string(),
            path: "/user".to_string(),
            headers: Default::default(),
            query_params: Default::default(),
            cookies: Default::default(),
            body: None,
            form_data: None,
            content_type: None,
        },
        expected_response: HttpExpectedResponse {
            status_code: 200,
            body: Some(serde_json::json!({"id": 1})),
            body_partial: None,
            headers: Default::default(),
            validation_errors: None,
        },
    });

    let e2e_config = E2eConfig::default();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs = [];
    let enums = [];
    let adapters = [];
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &adapters,
        &config,
        &type_defs,
        &enums,
        &[],
        &[],
    );

    assert!(
        !output.contains("Directory.current.uri.resolve('../rust/Cargo.toml')"),
        "the standalone-mock-server branch must not re-derive a cwd-relative manifest path, got:\n{output}"
    );
    assert!(
        output.contains("import 'e2e_helpers.dart';"),
        "a file that spawns the standalone mock server must import the shared helper, got:\n{output}"
    );
    assert!(
        output.contains("final _handle = await startMockServer();"),
        "the standalone-mock-server branch must delegate to the shared helper, got:\n{output}"
    );
    assert!(
        output.contains("await _mockServerHandle?.stop();"),
        "tearDownAll must stop a helper-owned mock server, got:\n{output}"
    );
}

/// A crate whose dart e2e fixtures never spawn the standalone mock server (e.g. one that
/// only ever runs the server-pattern `app_harness.dart` branch, or has no HTTP/`mock_url`
/// fixtures at all) must regenerate to an unchanged file set: `e2e_helpers.dart` must not
/// appear as a new, unreferenced file. Companion to
/// `dart_standalone_mock_server_spawn_delegates_to_the_shared_helper` above, which covers
/// the crate that DOES need it.
#[test]
fn crate_with_no_standalone_mock_server_fixture_does_not_gain_an_unreferenced_helper_file() {
    use super::E2eCodegen;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::FixtureGroup;

    let group = FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture("plain_fixture")],
    };
    let e2e_config = E2eConfig::default();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let files = super::DartE2eCodegen
        .generate(&[group], &e2e_config, &config, &[], &[], &[], &[])
        .expect("dart e2e generation must succeed for a plain fixture");

    assert!(
        !files.iter().any(|f| f.path.ends_with("e2e_helpers.dart")),
        "a crate with no standalone-mock-server fixture must not gain e2e_helpers.dart, got files: {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
}

/// `dart test` runs every generated file's `setUpAll`/`tearDownAll` sequentially inside one
/// OS process, and `Directory.current` is process-global, not per-isolate. A file whose
/// `setUpAll` chdirs into `test_documents` must restore the original cwd in `tearDownAll`,
/// or the mutation leaks into whichever file `dart test` runs next -- that file's own
/// relative `'../../test_documents'` (or a standalone-mock-server file's `_findRepoRoot()`
/// walk) then resolves against an already-shifted cwd instead of the real starting
/// directory. Regression for `Bad state: could not locate repository root from
/// <sibling-of-repo>/test_documents`. ~keep
#[test]
fn dart_chdir_setup_all_restores_original_cwd_in_teardown() {
    use crate::e2e::config::{ArgMapping, E2eConfig};

    let mut fixture = make_fixture("extract_from_path");
    fixture.input = serde_json::json!({ "path": "docx/fake.docx" });

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "extractFile".into();
    e2e_config.call.args = vec![ArgMapping {
        name: "path".to_string(),
        field: "input.path".to_string(),
        arg_type: "file_path".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(
        output.contains("Directory.current = _dir;"),
        "expected the fixture to trigger the test_documents chdir, got:\n{output}"
    );
    assert!(
        output.contains("String? _originalCwd;"),
        "the pre-chdir cwd must be captured in a variable visible to tearDownAll, got:\n{output}"
    );
    assert!(
        output.contains("_originalCwd = Directory.current.path;"),
        "setUpAll must capture the cwd before mutating it, got:\n{output}"
    );
    let setup_all_end = output
        .find("_originalCwd = Directory.current.path;")
        .expect("capture line must be present");
    let chdir_pos = output
        .find("Directory.current = _dir;")
        .expect("chdir line must be present");
    assert!(
        setup_all_end < chdir_pos,
        "the cwd must be captured before it is mutated, got:\n{output}"
    );
    assert!(
        output.contains("if (_cwd != null) Directory.current = _cwd;"),
        "tearDownAll must restore the captured cwd so it does not leak into the next \
         `dart test`-run file, got:\n{output}"
    );
}
