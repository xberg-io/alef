use alef::backends::{java::JavaBackend, pyo3::Pyo3Backend};
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, TypeRef};

fn config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "java", "ffi"]
[[crates]]
name = "fixture"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_fixture"
[crates.python.stubs]
output = "python/"
[crates.java]
package = "fixture"
[crates.ffi]
prefix = "fixture"
"#,
    )
    .unwrap();
    config.resolve().unwrap().remove(0)
}

fn surface() -> ApiSurface {
    serde_json::from_value(serde_json::json!({
        "crate_name": "fixture", "version": "0.1.0", "types": [], "functions": [], "enums": [],
        "errors": [
            {"name": "RequestError", "rust_path": "fixture::RequestError", "doc": "", "variants": [
                {"name": "TooManyRedirects", "message_template": "loop", "doc": "", "is_unit": true}]},
            {"name": "PolicyError", "exception_base": "RequestError", "rust_path": "fixture::PolicyError", "doc": "", "variants": [
                {"name": "TooManyRedirects", "message_template": "loop", "doc": "", "is_unit": true},
                {"name": "Denied", "message_template": "denied", "doc": "", "is_unit": true}]}
        ]
    })).unwrap()
}

#[test]
fn generated_python_preserves_both_generic_catches_and_the_specific_class() {
    let api = surface();
    let files = Pyo3Backend.generate_bindings(&api, &config()).unwrap();
    let source = files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        source.contains("create_exception!(_fixture, PolicyError, RequestError)"),
        "{source}"
    );
    assert!(
        source.contains("create_exception!(_fixture, TooManyRedirectsError, PolicyError)"),
        "{source}"
    );
    assert_eq!(
        source
            .matches("create_exception!(_fixture, TooManyRedirectsError,")
            .count(),
        1
    );
    let stubs = Pyo3Backend.generate_type_stubs(&api, &config()).unwrap();
    let stub = stubs
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(stub.contains("class PolicyError(RequestError): ..."));
    assert!(stub.contains("class TooManyRedirectsError(PolicyError): ..."));
}

#[test]
fn generated_java_preserves_both_generic_catches_and_the_specific_class() {
    let files = JavaBackend.generate_bindings(&surface(), &config()).unwrap();
    let content = |name: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(name))
            .unwrap()
            .content
            .as_str()
    };
    assert!(content("PolicyErrorException.java").contains("extends RequestErrorException"));
    assert!(content("TooManyRedirectsException.java").contains("extends PolicyErrorException"));
    assert_eq!(
        files
            .iter()
            .filter(|file| file.path.ends_with("TooManyRedirectsException.java"))
            .count(),
        1
    );
}

#[test]
fn extractor_records_the_opt_in_error_parent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
#[derive(thiserror::Error)]
pub enum RequestError { #[error("loop")] TooManyRedirects }
#[derive(thiserror::Error)]
#[cfg_attr(alef, alef(exception_base = "RequestError"))]
pub enum PolicyError { #[error("loop")] TooManyRedirects }
"#,
    )
    .unwrap();
    let api = alef::extract::extractor::extract(&[&path], "fixture", "0.1.0", None).unwrap();
    assert_eq!(
        api.errors
            .iter()
            .find(|error| error.name == "PolicyError")
            .unwrap()
            .exception_base
            .as_deref(),
        Some("RequestError")
    );
}

fn with_bases(bases: serde_json::Value) -> ApiSurface {
    let mut value = serde_json::to_value(surface()).unwrap();
    for error in value["errors"].as_array_mut().unwrap() {
        error["exception_base"] = bases
            .get(error["name"].as_str().unwrap())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
    }
    serde_json::from_value(value).unwrap()
}

#[test]
fn unknown_exception_parent_is_rejected_before_emitting_code() {
    let error = Pyo3Backend
        .generate_bindings(&with_bases(serde_json::json!({"PolicyError": "Missing"})), &config())
        .unwrap_err();
    assert!(error.to_string().contains("unknown exception parent"), "{error}");
}

#[test]
fn cyclic_exception_parents_are_rejected_before_emitting_code() {
    let error = JavaBackend
        .generate_bindings(
            &with_bases(serde_json::json!({"PolicyError": "RequestError", "RequestError": "PolicyError"})),
            &config(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("exception hierarchy cycle"), "{error}");
}

#[test]
fn unrelated_shared_variant_parents_are_rejected_when_opted_in() {
    let mut value = serde_json::to_value(surface()).unwrap();
    for error in value["errors"].as_array_mut().unwrap() {
        error["exception_base"] = serde_json::json!("RootError");
    }
    value["errors"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "RootError", "rust_path": "fixture::RootError", "variants": [], "doc": ""}));
    let api = serde_json::from_value(value).unwrap();
    let error = Pyo3Backend.generate_bindings(&api, &config()).unwrap_err();
    assert!(error.to_string().contains("incompatible exception parents"), "{error}");
}

#[test]
fn generated_java_runtime_keeps_legacy_and_new_generic_catches() {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("fixture");
    std::fs::create_dir(&package).unwrap();
    let mut sources = Vec::new();
    for file in JavaBackend.generate_bindings(&surface(), &config()).unwrap() {
        let name = file.path.file_name().unwrap().to_string_lossy();
        if name.ends_with("Exception.java") {
            let path = package.join(name.as_ref());
            std::fs::write(&path, file.content).unwrap();
            sources.push(path);
        }
    }
    let probe = package.join("HierarchyProbe.java");
    std::fs::write(
        &probe,
        r#"package fixture;
public class HierarchyProbe {
    public static void main(String[] args) {
        Object legacy = new TooManyRedirectsException("legacy");
        if (!(legacy instanceof PolicyErrorException)) throw new AssertionError("legacy catch lost");
        if (!(legacy instanceof RequestErrorException)) throw new AssertionError("new generic catch lost");
        Object denied = new DeniedException("denied");
        if (!(denied instanceof PolicyErrorException)) throw new AssertionError("policy catch lost");
        if (!(denied instanceof RequestErrorException)) throw new AssertionError("root catch lost");
        System.out.println("HIERARCHY_ASSERTIONS=4");
    }
}"#,
    )
    .unwrap();
    sources.push(probe);
    let compile = std::process::Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = std::process::Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "fixture.HierarchyProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "HIERARCHY_ASSERTIONS=4");
}

#[cfg(unix)]
#[test]
fn generated_native_python_runtime_keeps_legacy_and_new_generic_catches() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("fixture");
    let binding = directory.path().join("fixture-py");
    write_python_core(&core);
    write_python_binding(&binding);
    let target = std::env::var_os("ALEF_HIERARCHY_RUNTIME_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().join("target"));
    build_python_library(&binding, &target);
    let probe = r#"import _fixture as m
for callback in [m.throw_new, m.throw_legacy]:
    try:
        callback()
    except m.TooManyRedirectsError as error:
        assert isinstance(error, m.PolicyError), 'legacy PolicyError catch lost'
        assert isinstance(error, m.RequestError), 'new RequestError catch lost'
    else:
        raise AssertionError('error converter did not raise')
assert isinstance(m.DeniedError('denied'), m.PolicyError)
assert isinstance(m.DeniedError('denied'), m.RequestError)
print('HIERARCHY_ASSERTIONS=6')
"#;
    let output = std::process::Command::new("python3")
        .args(["-c", probe])
        .env("PYTHONPATH", &binding)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "HIERARCHY_ASSERTIONS=6");
}

#[cfg(unix)]
fn write_python_core(core: &std::path::Path) {
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(core.join("src/lib.rs"), r#"
#[derive(Debug)] pub enum RequestError { TooManyRedirects }
#[derive(Debug)] pub enum PolicyError { TooManyRedirects, Denied }
impl std::fmt::Display for RequestError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "loop") } }
impl std::fmt::Display for PolicyError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "legacy") } }
pub fn throw_new() -> Result<(), RequestError> { Err(RequestError::TooManyRedirects) }
pub fn throw_legacy() -> Result<(), PolicyError> { Err(PolicyError::TooManyRedirects) }
"#).unwrap();
}

#[cfg(unix)]
fn write_python_binding(binding: &std::path::Path) {
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(binding.join("Cargo.toml"), "[package]\nname='fixture-py'\nversion='0.1.0'\nedition='2024'\n[lib]\nname='_fixture'\ncrate-type=['cdylib']\n[dependencies]\nfixture={path='../fixture'}\nserde={version='1',features=['derive']}\nserde_json='1'\npyo3={version='0.29',features=['extension-module','abi3-py310']}\npyo3-async-runtimes={version='0.29',features=['tokio-runtime']}\ntokio={version='1',features=['full']}\nfutures='0.3'\n[build-dependencies]\npyo3-build-config='0.29'\n").unwrap();
    std::fs::write(
        binding.join("build.rs"),
        "fn main() { pyo3_build_config::add_extension_module_link_args(); }\n",
    )
    .unwrap();
    let mut api = surface();
    api.functions = [("throw_new", "RequestError"), ("throw_legacy", "PolicyError")]
        .into_iter()
        .map(|(name, error)| alef::core::ir::FunctionDef {
            name: name.to_string(),
            rust_path: format!("fixture::{name}"),
            error_type: Some(error.to_string()),
            return_type: TypeRef::Unit,
            ..Default::default()
        })
        .collect();
    let source = Pyo3Backend
        .generate_bindings(&api, &config())
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
}

#[cfg(unix)]
fn build_python_library(binding: &std::path::Path, target: &std::path::Path) {
    let built = std::process::Command::new("cargo")
        .args(["build", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let name = if cfg!(target_os = "macos") {
        "lib_fixture.dylib"
    } else {
        "lib_fixture.so"
    };
    std::fs::copy(target.join("debug").join(name), binding.join("_fixture.abi3.so")).unwrap();
}

#[test]
fn generated_csharp_preserves_both_generic_catches_and_the_specific_class() {
    let files = alef::backends::csharp::CsharpBackend
        .generate_bindings(&surface(), &config())
        .unwrap();
    let content = |name: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(name))
            .unwrap()
            .content
            .as_str()
    };
    assert!(content("PolicyErrorException.cs").contains(": RequestErrorException"));
    assert!(content("TooManyRedirectsException.cs").contains(": PolicyErrorException"));
    assert_eq!(
        files
            .iter()
            .filter(|file| file.path.ends_with("TooManyRedirectsException.cs"))
            .count(),
        1
    );
}

#[test]
fn an_unannotated_error_keeps_its_original_exception_shape() {
    let api = with_bases(serde_json::json!({}));
    assert!(api.errors.iter().all(|error| {
        !serde_json::to_value(error)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("exception_base")
    }));
    let files = Pyo3Backend.generate_bindings(&api, &config()).unwrap();
    let source = files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(source.contains("create_exception!(_fixture, PolicyError, pyo3::exceptions::PyException)"));
    // ~keep Flat emission is alphabetically first-wins: PolicyError precedes RequestError.
    assert!(source.contains("create_exception!(_fixture, TooManyRedirectsError, PolicyError)"));
}

#[test]
fn non_string_exception_metadata_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"#[derive(thiserror::Error)] #[cfg_attr(alef, alef(exception_base = 7))]
pub enum SampleError { #[error("refused")] Refused }"#,
    )
    .unwrap();
    let error = alef::extract::extractor::extract(&[&path], "fixture", "0.1.0", None).unwrap_err();
    assert!(
        error.to_string().contains("exception_base must be a string parent"),
        "{error}"
    );
}

#[test]
fn shared_exception_classes_reject_incompatible_introspection_constructor_layouts() {
    let mut api = surface();
    api.errors[0].methods.push(alef::core::ir::MethodDef {
        name: "status_code".into(),
        return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::U32),
        ..Default::default()
    });
    let error = JavaBackend
        .generate_bindings(&api, &config())
        .expect_err("shared classes must not lose an existing constructor layout")
        .to_string();
    assert!(
        error.contains("incompatible exception introspection")
            && error.contains("RequestError")
            && error.contains("PolicyError"),
        "{error}"
    );
}
