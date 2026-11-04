use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = r#"
[workspace]
alef_version = "__VERSION__"
languages = ["python", "dart", "swift"]

[[crates]]
name = "sample"
core_import = "sample"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
error_type = "Error"
error_constructor = "Error::from({msg})"

[[crates.trait_bridges]]
trait_name = "Counter"
ffi_skip_methods = ["checked_count"]

[crates.generate]
public_api = false

[extensions.python_host_aliases.crates.sample.traits.Counter]
"#;

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = 'sample'\nversion = '0.1.0'\nedition = '2024'\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("src/lib.rs"),
        include_str!("fixtures/python_host_aliases/core.rs"),
    )
    .unwrap();
    root
}

fn generate(root: &Path, alias: &str) -> Output {
    generate_with_config(root, CONFIG, alias)
}

fn generate_with_config(root: &Path, config: &str, alias: &str) -> Output {
    std::fs::write(
        root.join("alef.toml"),
        config.replace("__VERSION__", env!("CARGO_PKG_VERSION")) + alias,
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_alef"))
        .args(["generate", "--skip-frb", "--skip-compile", "--jobs", "1"])
        .current_dir(root)
        .output()
        .unwrap()
}

fn output_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn references(root: &Path) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .args([
            "docs",
            "--lang",
            "python,rust",
            "--output",
            "reference",
            "--skip-snippet-validation",
            "--jobs",
            "1",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    (
        std::fs::read_to_string(root.join("reference/api-python.md")).unwrap(),
        std::fs::read_to_string(root.join("reference/api-rust.md")).unwrap(),
    )
}

fn assert_host_bridges(root: &Path, exposes_checked_count: bool) {
    for (path, count, describe, checked) in [
        (
            "packages/dart/lib/src/traits.dart",
            " count(",
            " describe(",
            " checkedCount(",
        ),
        (
            "packages/dart/rust/src/lib.rs",
            "fn count(",
            "fn describe(",
            "fn checked_count(",
        ),
        (
            "packages/swift/Sources/RustBridge/SwiftCounterBridge.swift",
            "func count(",
            "func describe(",
            "func checkedCount(",
        ),
        (
            "packages/swift/rust/src/lib.rs",
            "fn count(",
            "fn describe(",
            "fn checked_count(",
        ),
    ] {
        let code = std::fs::read_to_string(root.join(path)).unwrap();
        assert!(code.contains(count), "{path} must retain the required callback");
        assert!(
            code.contains(describe),
            "{path} must retain the unrelated defaulted callback"
        );
        assert_eq!(
            code.contains(checked),
            exposes_checked_count,
            "{path} checked callback exposure"
        );
    }
}

fn python_bridge(root: &Path) -> String {
    let code = std::fs::read_to_string(root.join("crates/sample-py/src/lib.rs")).unwrap();
    assert!(code.contains("pub struct PyCounterBridge"));
    assert!(code.contains("match obj.getattr(\"checked_count\")"));
    assert!(code.contains("obj.getattr(\"count\")?"));
    code
}

#[test]
fn opted_in_alias_projects_only_its_defaulted_callback_out_of_dart_and_swift() {
    let legacy = fixture();
    let output = generate(legacy.path(), "checked_count = 'count'\n");
    assert!(output.status.success(), "{}", output_text(&output));
    assert_host_bridges(legacy.path(), true);
    let original_python = python_bridge(legacy.path());

    let scoped = fixture();
    let output = generate(
        scoped.path(),
        "checked_count = { fallback = 'count', python_only = true }\n",
    );
    assert!(output.status.success(), "{}", output_text(&output));
    assert_host_bridges(scoped.path(), false);
    assert_eq!(python_bridge(scoped.path()), original_python);

    let (legacy_python, _) = references(legacy.path());
    assert!(legacy_python.contains("###### checked_count()"));
    let (scoped_python, scoped_rust) = references(scoped.path());
    assert!(scoped_python.contains("###### count()"));
    assert!(scoped_python.contains("###### describe()"));
    assert!(
        !scoped_python.contains("###### checked_count()"),
        "the optional companion must not be presented as a public Python protocol method"
    );
    assert!(scoped_rust.contains("###### checked_count()"));
}

#[test]
fn opted_in_alias_refuses_to_project_a_required_rust_callback() {
    let root = fixture();
    let output = generate(root.path(), "alternate = { fallback = 'count', python_only = true }\n");
    assert!(!output.status.success(), "must reject a required callback");
    let report = output_text(&output);
    assert!(report.contains("callback must be defaulted and fallible"), "{report}");
    assert!(!root.path().join("packages/dart/lib/src/traits.dart").exists());
}

#[test]
fn private_callback_keeps_unrelated_inherent_method_with_same_name() {
    let config = CONFIG.replace("public_api = false", "public_api = true");
    for (alias, exposed) in [
        ("checked_count = 'count'\n", true),
        ("checked_count = { fallback = 'count', python_only = true }\n", false),
    ] {
        let root = fixture();
        let source = root.path().join("src/lib.rs");
        let core = std::fs::read_to_string(&source).unwrap();
        std::fs::write(
            source,
            core + "\n" + include_str!("fixtures/python_host_aliases/concrete.rs"),
        )
        .unwrap();
        let output = generate_with_config(root.path(), &config, alias);
        assert!(output.status.success(), "{}", output_text(&output));
        let protocol = std::fs::read_to_string(
            root.path()
                .join("packages/swift/Sources/RustBridge/SwiftCounterBridge.swift"),
        )
        .unwrap();
        assert_eq!(protocol.contains("func checkedCount("), exposed);
        let bindings = std::fs::read_to_string(root.path().join("packages/swift/rust/src/lib.rs")).unwrap();
        assert!(
            bindings.contains("pub fn widget_checked_count_from_json("),
            "unrelated Widget::checked_count must survive {alias}"
        );
        assert!(
            bindings.contains("pub fn implementer_checked_count_from_json("),
            "public Implementer::checked_count must survive {alias}"
        );
    }
}
