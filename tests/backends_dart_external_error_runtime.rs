use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use std::path::Path;
use std::process::Command;

const CORE: &str = include_str!("fixtures/dart_external_error_core.rs");

fn emitted_bridge(root: &Path, hidden_payloads: bool, error_name: &str) -> String {
    emitted_bridge_with_error_method(root, hidden_payloads, error_name, false)
}

/// The generator writes the conversion from the Dart mirror back to the core error only for an
/// error that has a bridged method, so `with_error_method` adds one to the extracted error.
fn emitted_bridge_with_error_method(
    root: &Path,
    hidden_payloads: bool,
    error_name: &str,
    with_error_method: bool,
) -> String {
    let extracted = CORE.replace("ConsumerError", error_name);
    let extracted = extracted
        .lines()
        .filter(|line| !line.contains("UNLISTED_FUTURE"))
        .collect::<Vec<_>>()
        .join("\n");
    let path = root.join("lib.rs");
    std::fs::write(&path, extracted).unwrap();
    let mut api = alef::extract::extractor::extract(&[&path], "consumer-core", "1.0.0", None).unwrap();
    assert_eq!(api.errors.len(), 1);
    assert_eq!(api.errors[0].variants.len(), 5);
    assert_eq!(api.errors[0].variants[0].fields[0].ty, TypeRef::String);
    assert_eq!(api.errors[0].variants[0].fields[0].original_type, None);
    if with_error_method {
        api.errors[0].methods.push(MethodDef {
            name: "is_retryable".into(),
            receiver: Some(ReceiverKind::Ref),
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            ..Default::default()
        });
    }
    if hidden_payloads {
        for variant in &mut api.errors[0].variants {
            for field in &mut variant.fields {
                field.binding_excluded = true;
            }
        }
    }
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"consumer-core\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = DartBackend
        .generate_bindings(&api, &config.resolve().unwrap()[0])
        .unwrap();
    files
        .into_iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content
}

fn without_frb_attributes(source: &str) -> String {
    let mut attribute = false;
    source
        .lines()
        .filter(|line| {
            let line = line.trim();
            if line == "#[frb]" {
                return false;
            }
            if line.starts_with("#[frb(") {
                attribute = !line.ends_with(")]");
                return false;
            }
            if attribute {
                attribute = !line.ends_with(")]");
                return false;
            }
            !line.starts_with("mod frb_generated;") && !line.contains("use flutter_rust_bridge::")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn successful(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn external_non_exhaustive_errors_preserve_borrowed_fields_and_unknown_failures() {
    let root = tempfile::tempdir().unwrap();
    let generated = emitted_bridge(root.path(), false, "ConsumerError");
    compile_consumer(root.path(), &generated, DRIVER, CORE);
    verify_dart_codec(root.path(), &generated, "ConsumerError");
}

#[test]
fn optional_string_error_fields_pass_clippy_without_an_identity_map() {
    let root = tempfile::tempdir().unwrap();
    let generated = emitted_bridge_with_error_method(root.path(), false, "ConsumerError", true);
    let arm = generated
        .split("ConsumerError::OptionalOwned { reason: f_reason } =>")
        .nth(1)
        .expect("the conversion back to the core error has an arm for the optional string field");
    let arm = arm.lines().take(4).collect::<Vec<_>>().join("\n");
    assert!(
        arm.contains("reason: f_reason.clone()"),
        "the optional string field is cloned directly by the conversion back to the core error:\n{arm}"
    );
    assert!(
        !generated.contains("as_ref().map(|v| v.clone())"),
        "an optional string field is cloned directly:\n{generated}"
    );
    compile_consumer(root.path(), &generated, "fn main() {}", CORE);
    let library = root.path().join("libconsumer_core.rlib");
    let consumer = root.path().join("consumer.rs");
    successful(
        Command::new("clippy-driver")
            .arg("--edition=2024")
            .arg(&consumer)
            .arg("--extern")
            .arg(format!("consumer_core={}", library.display()))
            .arg("--emit=metadata")
            .arg("--out-dir")
            .arg(root.path())
            .args(["-D", "warnings"]),
    );
}

#[test]
fn conversion_back_to_the_core_error_keeps_variant_and_text_of_borrowed_fields() {
    let root = tempfile::tempdir().unwrap();
    let generated = emitted_bridge_with_error_method(root.path(), false, "ConsumerError", true);
    compile_consumer(root.path(), &generated, ROUND_TRIP_DRIVER, CORE);
}

#[test]
fn external_non_exhaustive_error_with_hidden_payloads_compiles() {
    let root = tempfile::tempdir().unwrap();
    let generated = emitted_bridge(root.path(), true, "ConsumerError");
    compile_consumer(root.path(), &generated, UNIT_DRIVER, CORE);
}

#[test]
fn internal_carriers_do_not_collide_with_declared_error_names() {
    for error_name in ["NativeFailure", "InvalidInput"] {
        let root = tempfile::tempdir().unwrap();
        let generated = emitted_bridge(root.path(), false, error_name);
        let driver = DRIVER.replace("ConsumerError", error_name).replace(
            &format!("AlefBridgeErrorWire::{error_name} {{ message }}"),
            &format!("AlefBridgeErrorWire::{error_name}1 {{ message }}"),
        );
        let core = CORE.replace("ConsumerError", error_name);
        compile_consumer(root.path(), &generated, &driver, &core);
        verify_dart_codec(root.path(), &generated, error_name);
    }
}

#[allow(clippy::print_stderr, reason = "report toolchain-dependent runtime census")]
fn verify_dart_codec(root: &Path, generated: &str, error_name: &str) {
    if Command::new("dart").arg("--version").output().is_err() {
        eprintln!("SKIP Dart codec runtime: dart is unavailable");
        return;
    }
    let arms = codec_carriers(error_name);
    let mut source = format!("class {error_name} {{}}\nsealed class AlefBridgeErrorWire {{}}\n");
    for (name, _, ty, field) in &arms {
        source.push_str(&format!("final class AlefBridgeErrorWire_{name} extends AlefBridgeErrorWire {{ final {ty} {field}; AlefBridgeErrorWire_{name}({{required this.{field}}}); }}\n"));
    }
    for (attribute, function, argument, result) in [
        ("rust2dart", "decode", "AlefBridgeErrorWire", "Object"),
        ("dart2rust", "encode", "Object", "AlefBridgeErrorWire"),
    ] {
        let expression = codec_expression(generated, attribute, &arms);
        source.push_str(&format!("{result} {function}({argument} value) => {expression};\n"));
    }
    source.push_str(&DART_DRIVER.replace("ConsumerError", error_name));
    let path = root.join("codec.dart");
    std::fs::write(&path, source).unwrap();
    successful(Command::new("dart").arg("run").arg(path));
}

fn codec_carriers(error_name: &str) -> [(&str, String, &str, &str); 3] {
    let factory = format!("{}{}", error_name[..1].to_lowercase(), &error_name[1..]);
    let validation = if error_name == "InvalidInput" {
        "InvalidInput1"
    } else {
        "InvalidInput"
    };
    let failure = if error_name == "NativeFailure" {
        "NativeFailure1"
    } else {
        "NativeFailure"
    };
    [
        (error_name, factory, "Object", "error"),
        (
            validation,
            format!("invalidInput{}", if validation.ends_with('1') { "1" } else { "" }),
            "String",
            "message",
        ),
        (
            failure,
            format!("nativeFailure{}", if failure.ends_with('1') { "1" } else { "" }),
            "String",
            "message",
        ),
    ]
}

fn codec_expression(generated: &str, attribute: &str, arms: &[(&str, String, &str, &str)]) -> String {
    let line = generated
        .lines()
        .find(|line| line.starts_with(&format!("#[frb({attribute}(")))
        .unwrap();
    let literal = line.split("dart_code = ").nth(1).unwrap().strip_suffix("))]").unwrap();
    let mut expression: String = serde_json::from_str(literal).unwrap();
    expression = expression.replace("{}", "value");
    for (name, factory, _, _) in arms {
        expression = expression.replace(
            &format!("AlefBridgeErrorWire.{factory}("),
            &format!("AlefBridgeErrorWire_{name}("),
        );
    }
    expression
}

const DART_DRIVER: &str = r#"
void main() {
  final known = ConsumerError();
  if (!identical(decode(encode(known)), known)) throw StateError('known error lost');
  final unknown = decode(encode(StateError('new upstream failure')));
  if (unknown is! StateError || unknown.message != 'new upstream failure') throw StateError('unknown message lost');
  final invalid = decode(encode(ArgumentError('invalid input')));
  if (invalid is! ArgumentError || invalid.message != 'invalid input') throw StateError('validation lost');
  var rejected = false;
  try { encode(Object()); } on ArgumentError { rejected = true; }
  if (!rejected) throw StateError('unsupported error accepted');
}
"#;

fn compile_consumer(root: &Path, generated: &str, driver: &str, core_source: &str) {
    let core = root.join("core.rs");
    let library = root.join("libconsumer_core.rlib");
    std::fs::write(&core, core_source).unwrap();
    successful(
        Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-name",
                "consumer_core",
                "--crate-type",
                "rlib",
            ])
            .arg(&core)
            .arg("-o")
            .arg(&library),
    );
    let consumer = root.join("consumer.rs");
    std::fs::write(&consumer, format!("{}\n{}", without_frb_attributes(generated), driver)).unwrap();
    let executable = root.join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
    successful(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg(&consumer)
            .arg("--extern")
            .arg(format!("consumer_core={}", library.display()))
            .arg("-o")
            .arg(&executable),
    );
    successful(&mut Command::new(executable));
}

const DRIVER: &str = r#"
fn main() {
    match request(0) {
        Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::Denied { reason } })) => assert_eq!(reason, "private_network"),
        _ => panic!("borrowed field lost its typed variant"),
    }
    match request(1) {
        Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::Owned { reason } })) => assert_eq!(reason, "owned payload"),
        _ => panic!("owned field lost its typed variant"),
    }
    for (case, expected) in [(2, None), (3, Some("borrowed payload"))] {
        match request(case) {
            Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::OptionalBorrowed { reason } })) => assert_eq!(reason.as_deref(), expected),
            _ => panic!("optional borrowed field lost its typed variant"),
        }
    }
    for (case, expected) in [(4, None), (5, Some("owned optional payload"))] {
        match request(case) {
            Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::OptionalOwned { reason } })) => assert_eq!(reason.as_deref(), expected),
            _ => panic!("optional owned field lost its typed variant"),
        }
    }
    match request(7) {
        Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::Error })) => {}
        _ => panic!("unit error must remain typed"),
    }
    match request(6) {
        Err(AlefBridgeError(AlefBridgeErrorWire::NativeFailure { message })) => assert_eq!(message, "new upstream failure"),
        _ => panic!("unlisted future variant must remain a native failure"),
    }
    match request(-1) {
        Err(AlefBridgeError(AlefBridgeErrorWire::InvalidInput { message })) => assert!(message.contains("u8::MAX")),
        _ => panic!("invalid input must retain its own error"),
    }
}
"#;

const ROUND_TRIP_DRIVER: &str = r#"
fn main() {
    let denied: consumer_core::ConsumerError = (&ConsumerError::Denied { reason: "private_network".into() }).into();
    match denied {
        consumer_core::ConsumerError::Denied { reason } => assert_eq!(reason, "private_network"),
        other => panic!("borrowed field changed variant: {other:?}"),
    }
    let again: consumer_core::ConsumerError = (&ConsumerError::Denied { reason: "private_network".into() }).into();
    match again {
        consumer_core::ConsumerError::Denied { reason } => assert_eq!(reason, "private_network"),
        other => panic!("borrowed field changed variant on the second conversion: {other:?}"),
    }
    for expected in [None, Some("borrowed payload")] {
        let mirror = ConsumerError::OptionalBorrowed { reason: expected.map(String::from) };
        match consumer_core::ConsumerError::from(&mirror) {
            consumer_core::ConsumerError::OptionalBorrowed { reason } => assert_eq!(reason, expected),
            other => panic!("optional borrowed field changed variant: {other:?}"),
        }
    }
    let owned: consumer_core::ConsumerError = (&ConsumerError::Owned { reason: "owned payload".into() }).into();
    match owned {
        consumer_core::ConsumerError::Owned { reason } => assert_eq!(reason, "owned payload"),
        other => panic!("owned field changed variant: {other:?}"),
    }
}
"#;

const UNIT_DRIVER: &str = r#"
fn main() {
    for case in 0..=5 {
        let expected = match case {
            0 => ConsumerError::Denied,
            1 => ConsumerError::Owned,
            2 | 3 => ConsumerError::OptionalBorrowed,
            _ => ConsumerError::OptionalOwned,
        };
        match request(case) {
            Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error })) => assert_eq!(std::mem::discriminant(&error), std::mem::discriminant(&expected)),
            _ => panic!("hidden payload must preserve known variant"),
        }
    }
    match request(7) {
        Err(AlefBridgeError(AlefBridgeErrorWire::ConsumerError { error: ConsumerError::Error })) => {}
        _ => panic!("unit error must remain typed"),
    }
    match request(6) {
        Err(AlefBridgeError(AlefBridgeErrorWire::NativeFailure { message })) => assert_eq!(message, "new upstream failure"),
        _ => panic!("hidden payload must preserve unknown failure message"),
    }
}
"#;
