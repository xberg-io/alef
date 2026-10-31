//! External call sites keep the pre-shared-family public generator signatures.

use alef::codegen::error_gen::{
    gen_csharp_error_types, gen_java_error_types, gen_java_error_types_with_parent, gen_php_error_converter,
};
use alef::core::ir::{ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn legacy_public_generator_calls_remain_source_compatible() {
    let error = ErrorDef {
        name: "StorageError".into(),
        rust_path: "demo::StorageError".into(),
        variants: vec![ErrorVariant {
            name: "Missing".into(),
            is_unit: true,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let java = gen_java_error_types(&error, "demo", "Demo");
    let java_parent = gen_java_error_types_with_parent(&error, "demo", "Demo", None);
    let csharp = gen_csharp_error_types(&error, "Demo", None, &[]);
    let php = gen_php_error_converter(&error, "demo");
    assert_eq!(java.len(), 2);
    assert_eq!(java, java_parent);
    assert_eq!(csharp.len(), 2);
    assert!(php.contains("StorageError::Missing"), "{php}");
}

const JAVA_INFRASTRUCTURE: &str = r#"package demo;
public class DemoException extends Exception {
  public DemoException(String message) { super(message); }
  public DemoException(String message, Throwable cause) { super(message, cause); }
}
"#;

const JAVA_PROBE: &str = r#"package demo;
public class LegacyHelperProbe {
  private static int check(boolean condition, String description) {
    if (!condition) throw new AssertionError(description);
    return 1;
  }
  public static void main(String[] args) {
    Throwable cause = new IllegalStateException("origin");
    MissingException plain = new MissingException("plain");
    MissingException nested = new MissingException("wrapped", cause);
    StorageErrorException base = new StorageErrorException("base", cause);
    int checks = 0;
    checks += check(plain.getClass() == MissingException.class, "variant identity");
    checks += check("plain".equals(plain.getMessage()), "message constructor");
    checks += check(plain.getCause() == null, "default cause");
    checks += check(plain instanceof StorageErrorException, "family inheritance");
    checks += check(plain instanceof DemoException, "infrastructure inheritance");
    checks += check("wrapped".equals(nested.getMessage()), "variant cause message");
    checks += check(nested.getCause() == cause, "variant cause identity");
    checks += check("base".equals(base.getMessage()), "base cause message");
    checks += check(base.getCause() == cause, "base cause identity");
    // INTROSPECTION_CHECKS
    if (checks != EXPECTED_CHECKS) throw new AssertionError("check census: " + checks);
    System.out.println("LEGACY_HELPER_METHODS=WITH_METHODS CHECKS=" + checks);
  }
}
"#;

const JAVA_INTROSPECTION_CHECKS: &str = r#"
    MissingException payload = new MissingException("payload", 201);
    StorageErrorException basePayload = new StorageErrorException("base payload", 202);
    checks += check(plain.getStatusCode() == 0, "message constructor default");
    checks += check(nested.getStatusCode() == 0, "cause constructor default");
    checks += check(payload.getStatusCode() == 201, "variant payload constructor");
    checks += check("payload".equals(payload.getMessage()), "variant payload message");
    checks += check(basePayload.getStatusCode() == 202, "base payload constructor");
"#;

fn legacy_java_error(with_methods: bool) -> ErrorDef {
    ErrorDef {
        name: "StorageError".into(),
        rust_path: "demo::StorageError".into(),
        variants: vec![ErrorVariant {
            name: "Missing".into(),
            is_unit: true,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: if with_methods {
            vec![MethodDef {
                name: "status_code".into(),
                return_type: TypeRef::Primitive(PrimitiveType::U16),
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            }]
        } else {
            vec![]
        },
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn legacy_probe_source(with_methods: bool) -> String {
    assert_eq!(JAVA_PROBE.matches("// INTROSPECTION_CHECKS").count(), 1);
    assert_eq!(JAVA_PROBE.matches("EXPECTED_CHECKS").count(), 1);
    assert_eq!(JAVA_PROBE.matches("WITH_METHODS").count(), 1);
    JAVA_PROBE
        .replace(
            "// INTROSPECTION_CHECKS",
            if with_methods { JAVA_INTROSPECTION_CHECKS } else { "" },
        )
        .replace("EXPECTED_CHECKS", if with_methods { "14" } else { "9" })
        .replace("WITH_METHODS", if with_methods { "true" } else { "false" })
}

fn write_legacy_java_fixture(directory: &Path, with_methods: bool) -> Vec<PathBuf> {
    let package = directory.join("demo");
    std::fs::create_dir(&package).expect("create owned Java source directory");
    let error = legacy_java_error(with_methods);
    let mut files = gen_java_error_types(&error, "demo", "Demo");
    assert_eq!(files, gen_java_error_types_with_parent(&error, "demo", "Demo", None));
    assert_eq!(
        files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        ["StorageErrorException", "MissingException"]
    );
    assert!(files.iter().all(|(_, source)| !source.contains(" implements ")));
    files.push(("DemoException".into(), JAVA_INFRASTRUCTURE.into()));
    files.push(("LegacyHelperProbe".into(), legacy_probe_source(with_methods)));
    assert_eq!(
        files.len(),
        4,
        "base, variant, caller-owned infrastructure and probe only"
    );
    files
        .into_iter()
        .map(|(name, source)| {
            let path = package.join(format!("{name}.java"));
            std::fs::write(&path, source).expect("write generated legacy Java source");
            path
        })
        .collect()
}

fn compile_legacy_java(sources: &[PathBuf], classes: &Path) -> Output {
    let javac = which::which("javac").expect("managed JDK compiler is required, never skipped");
    Command::new(javac)
        .arg("-XDrawDiagnostics")
        .arg("-classpath")
        .arg(classes)
        .arg("-d")
        .arg(classes)
        .args(sources)
        .output()
        .expect("compile actual standalone legacy Java helper output")
}

fn assert_java_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reject_unprovided_interface(sources: &[PathBuf], classes: &Path) {
    let base = &sources[0];
    let source = std::fs::read_to_string(base).expect("read generated base before poisoning");
    let declaration = "public class StorageErrorException extends DemoException {";
    assert_eq!(source.matches(declaration).count(), 1);
    std::fs::write(
        base,
        source.replace(
            declaration,
            concat!(
                "public class StorageErrorException extends DemoException",
                " implements LegacyFamilyMarkerMustNotExist {"
            ),
        ),
    )
    .expect("poison only the generated base's interface declaration");
    let rejected = compile_legacy_java(sources, classes);
    assert!(!rejected.status.success(), "unprovided interface must fail compilation");
    let diagnostics = String::from_utf8_lossy(&rejected.stderr);
    let errors = diagnostics
        .lines()
        .filter(|line| line.contains("compiler.err."))
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 1, "exactly the poisoned symbol must fail: {diagnostics}");
    assert!(errors[0].contains("compiler.err.cant.resolve"), "{diagnostics}");
    assert!(errors[0].contains("LegacyFamilyMarkerMustNotExist"), "{diagnostics}");
}

#[test]
fn legacy_java_helpers_compile_and_run_without_unprovided_family_interfaces() {
    let java = which::which("java").expect("managed JDK runtime is required, never skipped");
    let mut scenarios = 0;
    for with_methods in [false, true] {
        let directory = tempfile::tempdir().expect("owned Java fixture removed after each scenario");
        let classes = directory.path().join("classes");
        std::fs::create_dir(&classes).expect("create isolated classpath");
        let sources = write_legacy_java_fixture(directory.path(), with_methods);
        assert_java_success(&compile_legacy_java(&sources, &classes));
        let output = Command::new(&java)
            .arg("-cp")
            .arg(&classes)
            .arg("demo.LegacyHelperProbe")
            .output()
            .expect("execute compiled legacy Java constructors and getters");
        assert_java_success(&output);
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("UTF-8 probe output")
                .replace("\r\n", "\n"),
            format!(
                "LEGACY_HELPER_METHODS={with_methods} CHECKS={}\n",
                if with_methods { 14 } else { 9 }
            )
        );
        reject_unprovided_interface(&sources, &classes);
        scenarios += 1;
    }
    assert_eq!(
        scenarios, 2,
        "both legacy method shapes must compile, run and reject poison"
    );
}
