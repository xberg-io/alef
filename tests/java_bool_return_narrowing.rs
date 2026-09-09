#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use support::{compile_java, java_available, run_java, test_config, write_file};

fn generated_bindings() -> String {
    let boolean = TypeRef::Primitive(PrimitiveType::Bool);
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        functions: vec![
            FunctionDef {
                name: "is_ready".into(),
                return_type: boolean.clone(),
                ..Default::default()
            },
            FunctionDef {
                name: "remove_entry".into(),
                return_type: boolean.clone(),
                error_type: Some("String".into()),
                ..Default::default()
            },
        ],
        types: vec![TypeDef {
            name: "Probe".into(),
            rust_path: "test_lib::Probe".into(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "is_ready".into(),
                return_type: boolean,
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    JavaBackend
        .generate_bindings(&api, &test_config())
        .expect("Java bindings")
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn should_narrow_boolean_results_without_changing_panama_long_layouts() {
    let generated = generated_bindings();
    for symbol in ["TEST_IS_READY", "TEST_REMOVE_ENTRY", "TEST_PROBE_IS_READY"] {
        let expected = format!("(int)(long) NativeLib.{symbol}.invoke(");
        assert!(
            generated.contains(&expected),
            "missing narrowed result for {symbol}:\n{generated}"
        );
    }
    for symbol in ["TEST_IS_READY", "TEST_REMOVE_ENTRY"] {
        let declaration = generated
            .split_once(&format!("static final MethodHandle {symbol} ="))
            .expect("free bool handle declaration")
            .1
            .split_once("\n    );")
            .expect("complete handle declaration")
            .0;
        assert!(
            declaration.contains("FunctionDescriptor.of(ValueLayout.JAVA_LONG)"),
            "{symbol} must retain its widened JBR-compatible descriptor: {declaration}"
        );
    }
}

#[test]
fn should_ignore_dirty_upper_bits_in_the_emitted_boolean_expression() {
    if !java_available() {
        return;
    }
    let generated = generated_bindings();
    let statement = generated
        .lines()
        .find(|line| line.contains("NativeLib.TEST_IS_READY.invoke()"))
        .expect("generated bool invocation")
        .trim()
        .replace("NativeLib.TEST_IS_READY.invoke()", "Long.valueOf(value)");
    let source = include_str!("fixtures/java_bool_narrowing_main.java").replace("GENERATED_INVOCATION", &statement);
    let directory = tempfile::tempdir().expect("Java bool probe directory");
    write_file(directory.path(), "com/test/BoolNarrowingMain.java", &source);
    compile_java(directory.path(), &["com/test/BoolNarrowingMain.java"]);
    run_java(directory.path(), "com.test.BoolNarrowingMain");
}
