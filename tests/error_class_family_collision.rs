//! Unrelated error families that declare variants of one name share a single exception class
//! under the unqualified name. The class extends the first family, as it always has, and also
//! belongs to every other declaring family: Python lists them as bases, Ruby includes a module per
//! family, and PHP, C# and Java implement each family's marker interface. Its payload is the union
//! of the families' fields, nullable where a family lacks one.

use alef::backends::csharp::CsharpBackend;
use alef::backends::java::JavaBackend;
use alef::backends::magnus::MagnusBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::codegen::error_gen::{
    gen_php_error_converter_with_errors, gen_php_error_stubs, gen_php_error_types, php_error_class_names,
};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
#[cfg(unix)]
use alef::core::ir::FunctionDef;
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::process::Command;

#[path = "support/php_native.rs"]
mod php_native;

fn u32_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        ..Default::default()
    }
}

fn unit(name: &str) -> ErrorVariant {
    ErrorVariant {
        name: name.into(),
        is_unit: true,
        ..Default::default()
    }
}

fn with_fields(name: &str, fields: Vec<FieldDef>) -> ErrorVariant {
    ErrorVariant {
        name: name.into(),
        fields,
        ..Default::default()
    }
}

fn error(name: &str, variants: Vec<ErrorVariant>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        variants,
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn surface(errors: Vec<ErrorDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        errors,
        ..Default::default()
    }
}

/// The spikard shape: `GraphQLError` and `SchemaError` share two variant names with different
/// payloads, and each also has a variant no other family declares.
fn colliding_api() -> ApiSurface {
    surface(vec![
        error(
            "GraphQLError",
            vec![
                unit("ComplexityLimitExceeded"),
                unit("DepthLimitExceeded"),
                unit("Parse"),
            ],
        ),
        error(
            "SchemaError",
            vec![
                with_fields("ComplexityLimitExceeded", vec![u32_field("actual"), u32_field("limit")]),
                with_fields("DepthLimitExceeded", vec![u32_field("actual"), u32_field("limit")]),
                unit("Invalid"),
            ],
        ),
    ])
}

fn compatible_api() -> ApiSurface {
    surface(vec![
        error(
            "GraphQLError",
            vec![with_fields("ComplexityLimitExceeded", vec![u32_field("limit")])],
        ),
        error(
            "SchemaError",
            vec![with_fields("ComplexityLimitExceeded", vec![u32_field("limit")])],
        ),
    ])
}

fn config(languages: &str, sections: &str) -> ResolvedCrateConfig {
    let toml = format!(
        "[workspace]\nlanguages = [{languages}]\n\n[[crates]]\nname = \"demo\"\nsources = [\"src/lib.rs\"]\n\n{sections}"
    );
    let config: NewAlefConfig = toml::from_str(&toml).expect("valid config");
    config.resolve().expect("resolved config").remove(0)
}

fn python_config() -> ResolvedCrateConfig {
    config(
        "\"python\"",
        "[crates.python]\nmodule_name = \"_demo\"\n\n[crates.python.stubs]\noutput = \"python/\"\n",
    )
}

fn joined(files: &[GeneratedFile]) -> String {
    files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn file<'a>(files: &'a [GeneratedFile], name: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|file_name| file_name == name))
        .unwrap_or_else(|| panic!("missing {name}"))
        .content
}

fn count(files: &[GeneratedFile], name: &str) -> usize {
    files
        .iter()
        .filter(|file| file.path.file_name().is_some_and(|file_name| file_name == name))
        .count()
}

fn csharp(api: &ApiSurface) -> Vec<GeneratedFile> {
    let config = config(
        "\"csharp\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\n\n[crates.csharp]\nnamespace = \"Demo\"\n",
    );
    CsharpBackend.generate_bindings(api, &config).expect("csharp bindings")
}

fn java(api: &ApiSurface) -> Vec<GeneratedFile> {
    let config = config(
        "\"java\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\n\n[crates.java]\npackage = \"demo\"\n",
    );
    JavaBackend.generate_bindings(api, &config).expect("java bindings")
}

#[test]
fn php_keeps_one_unqualified_class_with_the_union_of_both_payloads() {
    let api = colliding_api();
    let types = gen_php_error_types(&api, "Demo", None).expect("colliding families must generate");
    assert_eq!(
        types.matches("pub struct PhpComplexityLimitExceededError {").count(),
        1,
        "{types}"
    );
    assert!(
        types.contains(
            "#[php(name = \"Demo\\\\ComplexityLimitExceededError\")]\n#[php(extends(PhpGraphQLError))]\n\
             #[php(implements(PhpInterfaceSchemaErrorInterface))]\n#[derive(Default)]\n\
             pub struct PhpComplexityLimitExceededError {\n    #[php(prop, name = \"actual\")]\n    \
             pub actual: Option<u32>,\n    #[php(prop, name = \"limit\")]\n    pub limit: Option<u32>,\n}"
        ),
        "{types}"
    );
    assert!(!types.contains("GraphQLComplexityLimitExceededError"), "{types}");
    assert!(!types.contains("SchemaComplexityLimitExceededError"), "{types}");
    let names = php_error_class_names(&api, None).expect("class names");
    assert_eq!(
        names,
        [
            "PhpGraphQLError",
            "PhpComplexityLimitExceededError",
            "PhpDepthLimitExceededError",
            "PhpParseError",
            "PhpSchemaError",
            "PhpInvalidError",
        ]
    );
}

#[test]
fn php_families_implement_their_marker_interfaces() {
    let api = colliding_api();
    let types = gen_php_error_types(&api, "Demo", None).unwrap();
    for family in ["GraphQLError", "SchemaError"] {
        assert!(
            types.contains(&format!(
                "#[php_interface]\n#[php(name = \"Demo\\\\{family}Interface\")]\n\
                 #[php(extends(ce = ext_php_rs::zend::ce::throwable, stub = \"\\\\Throwable\"))]\n\
                 pub trait {family}Interface {{}}"
            )),
            "{types}"
        );
        assert!(
            types.contains(&format!(
                "#[php(implements(PhpInterface{family}Interface))]\n#[derive(Default)]\npub struct Php{family} {{"
            )),
            "{types}"
        );
    }
    let stubs = gen_php_error_stubs(&api, None).unwrap();
    for expected in [
        "interface GraphQLErrorInterface extends \\Throwable",
        "interface SchemaErrorInterface extends \\Throwable",
        "class SchemaError extends \\Exception implements SchemaErrorInterface",
        "class ComplexityLimitExceededError extends GraphQLError implements SchemaErrorInterface\n{\n    \
         public ?int $actual;\n    public ?int $limit;",
        "class InvalidError extends SchemaError\n",
    ] {
        assert!(stubs.contains(expected), "missing {expected:?} in:\n{stubs}");
    }
}

#[test]
fn php_converters_fill_the_fields_their_family_has_and_null_the_rest() {
    let api = colliding_api();
    let graphql = gen_php_error_converter_with_errors(&api.errors[0], &api.errors, "demo");
    assert!(
        graphql.contains(
            "demo::GraphQLError::ComplexityLimitExceeded => ext_php_rs::convert::IntoZval::into_zval(\
             PhpComplexityLimitExceededError {\n            actual: None,\n            limit: None,\n        }"
        ),
        "{graphql}"
    );
    let schema = gen_php_error_converter_with_errors(&api.errors[1], &api.errors, "demo");
    assert!(
        schema.contains(
            "demo::SchemaError::ComplexityLimitExceeded { actual, limit, .. } => \
             ext_php_rs::convert::IntoZval::into_zval(PhpComplexityLimitExceededError {\n            \
             actual: Some(*actual),\n            limit: Some(*limit),\n        }"
        ),
        "{schema}"
    );
}

#[test]
fn php_keeps_a_field_every_family_declares_non_nullable() {
    let types = gen_php_error_types(&compatible_api(), "Demo", None).unwrap();
    assert_eq!(types.matches("pub struct PhpComplexityLimitExceededError {").count(), 1);
    assert!(types.contains("    pub limit: u32,\n"), "{types}");
}

#[test]
fn php_still_rejects_a_field_two_families_declare_with_different_types() {
    let api = surface(vec![
        error("GraphQLError", vec![with_fields("Rejected", vec![u32_field("value")])]),
        error(
            "SchemaError",
            vec![with_fields(
                "Rejected",
                vec![FieldDef {
                    name: "value".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
            )],
        ),
    ]);
    let message = gen_php_error_types(&api, "Demo", None).unwrap_err().to_string();
    for expected in [
        "value",
        "RejectedError",
        "GraphQLError::Rejected",
        "SchemaError::Rejected",
    ] {
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn python_declares_the_shared_class_once_and_adds_every_family_as_a_base() {
    for api in [colliding_api(), compatible_api()] {
        let code = joined(&Pyo3Backend.generate_bindings(&api, &python_config()).unwrap());
        assert_eq!(
            code.matches("pyo3::create_exception!(_demo, ComplexityLimitExceededError, GraphQLError);")
                .count(),
            1,
            "{code}"
        );
        assert!(
            code.contains(
                "m.py().get_type::<ComplexityLimitExceededError>().setattr(\"__bases__\", \
                 (m.py().get_type::<GraphQLError>(), m.py().get_type::<SchemaError>(),))?;"
            ),
            "{code}"
        );
        assert!(!code.contains("GraphQLComplexityLimitExceededError"), "{code}");
        assert!(
            code.contains("demo::SchemaError::ComplexityLimitExceeded { .. } => ComplexityLimitExceededError::new_err"),
            "{code}"
        );
    }
}

#[test]
fn python_stubs_list_both_bases_and_the_union_of_attributes() {
    let stubs = joined(
        &Pyo3Backend
            .generate_type_stubs(&colliding_api(), &python_config())
            .unwrap(),
    );
    assert!(
        stubs.contains("class ComplexityLimitExceededError(GraphQLError, SchemaError):"),
        "{stubs}"
    );
    assert!(stubs.contains("actual: int | None"), "{stubs}");
    assert!(stubs.contains("class InvalidError(SchemaError)"), "{stubs}");
    let compatible = joined(
        &Pyo3Backend
            .generate_type_stubs(&compatible_api(), &python_config())
            .unwrap(),
    );
    assert!(compatible.contains("    limit: int\n"), "{compatible}");
}

#[test]
fn python_defaults_attributes_only_some_families_set_to_none() {
    let code = joined(
        &Pyo3Backend
            .generate_bindings(&colliding_api(), &python_config())
            .unwrap(),
    );
    for attribute in ["actual", "limit"] {
        assert!(
            code.contains(&format!(
                "m.py().get_type::<ComplexityLimitExceededError>().setattr(\"{attribute}\", m.py().None())?;"
            )),
            "{code}"
        );
    }
    let compatible = joined(
        &Pyo3Backend
            .generate_bindings(&compatible_api(), &python_config())
            .unwrap(),
    );
    assert!(
        !compatible.contains("setattr(\"limit\", m.py().None())"),
        "{compatible}"
    );
}

#[test]
fn ruby_includes_each_other_family_module_in_the_shared_class() {
    let code = joined(
        &MagnusBackend
            .generate_bindings(&colliding_api(), &config("\"ruby\"", ""))
            .unwrap(),
    );
    assert_eq!(
        code.matches("module.define_error(\"ComplexityLimitExceededError\", graph_ql_error_class)")
            .count(),
        1,
        "{code}"
    );
    assert!(code.contains("module Family; end"), "{code}");
    assert!(
        code.contains("(\"ComplexityLimitExceededError.include(SchemaError::Family)\",)"),
        "{code}"
    );
    assert!(!code.contains("GraphQLComplexityLimitExceededError"), "{code}");
}

#[test]
fn csharp_shared_class_implements_the_other_family_interface() {
    let files = csharp(&colliding_api());
    assert_eq!(count(&files, "ComplexityLimitExceededException.cs"), 1);
    assert!(
        file(&files, "ComplexityLimitExceededException.cs")
            .contains("public class ComplexityLimitExceededException : GraphQLErrorException, ISchemaError"),
    );
    assert!(file(&files, "SchemaErrorException.cs").contains(": DemoException, ISchemaError"));
    assert!(file(&files, "InvalidException.cs").contains("public class InvalidException : SchemaErrorException\n"));
    for family in ["GraphQLError", "SchemaError"] {
        let interface = file(&files, &format!("I{family}.cs"));
        assert!(
            interface.contains(&format!("public interface I{family}\n{{\n}}")),
            "{interface}"
        );
        assert!(interface.contains(&format!("when (e is I{family})")), "{interface}");
    }
}

#[test]
fn java_shared_class_implements_the_other_family_interface() {
    let files = java(&colliding_api());
    assert_eq!(count(&files, "ComplexityLimitExceededException.java"), 1);
    assert!(file(&files, "ComplexityLimitExceededException.java").contains(
        "public class ComplexityLimitExceededException extends GraphQLErrorException implements ISchemaError {"
    ),);
    assert!(file(&files, "SchemaErrorException.java").contains("implements ISchemaError {"));
    for family in ["GraphQLError", "SchemaError"] {
        assert!(file(&files, &format!("I{family}.java")).contains(&format!("public interface I{family} {{}}")));
    }
}

#[test]
fn unique_variant_names_keep_their_class_and_join_no_other_family() {
    let api = surface(vec![
        error("GraphQLError", vec![unit("Parse")]),
        error(
            "SchemaError",
            vec![with_fields("ComplexityLimitExceeded", vec![u32_field("limit")])],
        ),
    ]);
    let types = gen_php_error_types(&api, "Demo", None).unwrap();
    assert!(
        types.contains(
            "#[php(extends(PhpSchemaError))]\n#[derive(Default)]\npub struct PhpComplexityLimitExceededError {"
        ),
        "{types}"
    );
    let python = joined(&Pyo3Backend.generate_bindings(&api, &python_config()).unwrap());
    assert!(!python.contains("__bases__"), "{python}");
    let csharp = csharp(&api);
    assert!(
        file(&csharp, "ComplexityLimitExceededException.cs")
            .contains("public class ComplexityLimitExceededException : SchemaErrorException\n")
    );
    let ruby = joined(&MagnusBackend.generate_bindings(&api, &config("\"ruby\"", "")).unwrap());
    assert!(ruby.contains("class SchemaError\\n  module Family; end"), "{ruby}");
    assert!(!ruby.contains(".include("), "{ruby}");
}

/// Compiles the generated Java exceptions and checks the shared class against both families.
#[test]
fn generated_java_shared_class_is_an_instance_of_both_families() {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("demo");
    std::fs::create_dir(&package).unwrap();
    let mut sources = Vec::new();
    for generated in java(&colliding_api()) {
        let name = generated.path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("Exception.java") || (name.starts_with('I') && name.ends_with("Error.java")) {
            let path = package.join(&name);
            std::fs::write(&path, generated.content).unwrap();
            sources.push(path);
        }
    }
    let probe = package.join("SharedProbe.java");
    std::fs::write(
        &probe,
        r#"package demo;
public class SharedProbe {
    public static void main(String[] args) {
        Object shared = new ComplexityLimitExceededException("limit");
        if (!(shared instanceof GraphQLErrorException)) throw new AssertionError("base family lost");
        if (!(shared instanceof IGraphQLError)) throw new AssertionError("base interface lost");
        if (!(shared instanceof ISchemaError)) throw new AssertionError("second family missing");
        if (new ParseException("parse") instanceof ISchemaError) throw new AssertionError("unrelated member");
        if (!(new InvalidException("invalid") instanceof ISchemaError)) throw new AssertionError("family member lost");
        System.out.println("SHARED_ASSERTIONS=5");
    }
}"#,
    )
    .unwrap();
    sources.push(probe);
    let compile = Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "demo.SharedProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "SHARED_ASSERTIONS=5");
}

/// Builds the generated PyO3 module and checks `issubclass`, `except` and the attribute union.
#[cfg(unix)]
#[test]
fn generated_python_shared_class_is_a_subclass_of_both_families() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("demo");
    let binding = directory.path().join("demo-py");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        core.join("src/lib.rs"),
        r#"
#[derive(Debug)] pub enum GraphQLError { ComplexityLimitExceeded, DepthLimitExceeded, Parse }
#[derive(Debug)] pub enum SchemaError {
    ComplexityLimitExceeded { actual: u32, limit: u32 }, DepthLimitExceeded { actual: u32, limit: u32 }, Invalid,
}
impl std::fmt::Display for GraphQLError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "graphql") } }
impl std::fmt::Display for SchemaError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "schema") } }
pub fn fail_graphql() -> Result<(), GraphQLError> { Err(GraphQLError::ComplexityLimitExceeded) }
pub fn fail_schema() -> Result<(), SchemaError> { Err(SchemaError::ComplexityLimitExceeded { actual: 9, limit: 5 }) }
"#,
    )
    .unwrap();
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(binding.join("Cargo.toml"), "[package]\nname='demo-py'\nversion='0.1.0'\nedition='2024'\n[lib]\nname='_demo'\ncrate-type=['cdylib']\n[dependencies]\ndemo={path='../demo'}\nserde={version='1',features=['derive']}\nserde_json='1'\npyo3={version='0.29',features=['extension-module','abi3-py310']}\n[build-dependencies]\npyo3-build-config='0.29'\n").unwrap();
    std::fs::write(
        binding.join("build.rs"),
        "fn main() { pyo3_build_config::add_extension_module_link_args(); }\n",
    )
    .unwrap();
    let mut api = colliding_api();
    api.functions = [("fail_graphql", "GraphQLError"), ("fail_schema", "SchemaError")]
        .into_iter()
        .map(|(name, error)| FunctionDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            error_type: Some(error.to_string()),
            return_type: TypeRef::Unit,
            ..Default::default()
        })
        .collect();
    let source = Pyo3Backend
        .generate_bindings(&api, &python_config())
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
    let target = std::env::var_os("ALEF_HIERARCHY_RUNTIME_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().join("target"));
    let built = Command::new("cargo")
        .args(["build", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let name = if cfg!(target_os = "macos") {
        "lib_demo.dylib"
    } else {
        "lib_demo.so"
    };
    std::fs::copy(target.join("debug").join(name), binding.join("_demo.abi3.so")).unwrap();
    let probe = r#"import _demo as m
shared = m.ComplexityLimitExceededError
assert issubclass(shared, m.GraphQLError), shared.__mro__
assert issubclass(shared, m.SchemaError), shared.__mro__
assert not issubclass(m.ParseError, m.SchemaError)
for call, family, actual in [(m.fail_graphql, m.SchemaError, None), (m.fail_schema, m.GraphQLError, 9)]:
    try:
        call()
    except family as error:
        assert type(error) is shared, type(error)
        assert error.actual == actual, error.actual
    else:
        raise AssertionError('converter did not raise')
print('SHARED_ASSERTIONS=7')
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .env("PYTHONPATH", &binding)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "SHARED_ASSERTIONS=7");
}

/// Builds the generated ext-php-rs classes and checks the shared class against both families.
#[test]
#[ignore = "requires PHP development headers and the PHP executable"]
fn generated_php_shared_class_implements_both_family_interfaces() {
    let api = colliding_api();
    let source = format!(
        r#"
use ext_php_rs::prelude::*;
mod demo {{
    pub enum GraphQLError {{ ComplexityLimitExceeded, DepthLimitExceeded, Parse }}
    pub enum SchemaError {{ ComplexityLimitExceeded {{ actual: u32, limit: u32 }}, DepthLimitExceeded {{ actual: u32, limit: u32 }}, Invalid }}
    impl std::fmt::Display for GraphQLError {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str("graphql") }} }}
    impl std::fmt::Display for SchemaError {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str("schema") }} }}
}}
#[php_function]
fn fail_graphql() -> Result<(), ext_php_rs::exception::PhpException> {{ Err(graph_ql_error_to_php_err(demo::GraphQLError::ComplexityLimitExceeded)) }}
#[php_function]
fn fail_schema() -> Result<(), ext_php_rs::exception::PhpException> {{
    Err(schema_error_to_php_err(demo::SchemaError::ComplexityLimitExceeded {{ actual: 9, limit: 5 }}))
}}
#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {{
    module.interface::<PhpInterfaceGraphQLErrorInterface>().interface::<PhpInterfaceSchemaErrorInterface>()
        {}
        .function(wrap_function!(fail_graphql)).function(wrap_function!(fail_schema))
}}
{}
{}
{}
"#,
        php_error_class_names(&api, None)
            .unwrap()
            .iter()
            .map(|class| format!(".class::<{class}>()"))
            .collect::<String>(),
        gen_php_error_types(&api, "Demo", None).unwrap(),
        gen_php_error_converter_with_errors(&api.errors[0], &api.errors, "demo"),
        gen_php_error_converter_with_errors(&api.errors[1], &api.errors, "demo"),
    );
    let fixture = tempfile::tempdir().unwrap();
    let library = php_native::build_php_extension(fixture.path(), &source);
    let probe = r#"
foreach ([['fail_graphql', null], ['fail_schema', 9]] as [$call, $actual]) {
    try { $call(); throw new Exception('missing failure'); }
    catch (Demo\SchemaErrorInterface $error) {
        if (!($error instanceof Demo\ComplexityLimitExceededError) || !($error instanceof Demo\GraphQLError)
            || !($error instanceof Demo\GraphQLErrorInterface) || $error->actual !== $actual) {
            throw new Exception('incorrect shared class: ' . get_class($error) . ' ' . json_encode(get_object_vars($error)));
        }
    }
}
if ((new ReflectionClass('Demo\ParseError'))->implementsInterface('Demo\SchemaErrorInterface')) throw new Exception('unrelated member');
echo "2 cases passed\n";
"#;
    let runtime = Command::new("php")
        .arg("-d")
        .arg(format!("extension={}", library.display()))
        .arg("-r")
        .arg(probe)
        .output()
        .unwrap();
    assert!(
        runtime.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&runtime.stdout), "2 cases passed\n");
}

/// A family named like a Java infrastructure exception keeps the infrastructure class as its base,
/// which must still implement the family's marker interface.
#[test]
fn java_infrastructure_named_family_classes_implement_its_interface() {
    let api = surface(vec![error("ConversionError", vec![unit("Rejected")])]);
    let files = java(&api);
    assert!(
        file(&files, "ConversionErrorException.java").contains("implements IConversionError {"),
        "{}",
        file(&files, "ConversionErrorException.java")
    );
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("demo");
    std::fs::create_dir(&package).unwrap();
    let mut sources = Vec::new();
    for generated in &files {
        let name = generated.path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("Exception.java") || name == "IConversionError.java" {
            let path = package.join(&name);
            std::fs::write(&path, &generated.content).unwrap();
            sources.push(path);
        }
    }
    let probe = package.join("InfrastructureProbe.java");
    std::fs::write(
        &probe,
        r#"package demo;
public class InfrastructureProbe {
    public static void main(String[] args) {
        if (!(new RejectedException("rejected") instanceof IConversionError)) throw new AssertionError("variant lost");
        if (!(new ConversionErrorException("base") instanceof IConversionError)) throw new AssertionError("base lost");
        System.out.println("INFRASTRUCTURE=2");
    }
}"#,
    )
    .unwrap();
    sources.push(probe);
    let compile = Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "demo.InfrastructureProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "INFRASTRUCTURE=2");
}

fn with_type(mut api: ApiSurface, name: &str, is_trait: bool) -> ApiSurface {
    api.types.push(TypeDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        is_trait,
        ..Default::default()
    });
    api
}

#[test]
fn a_marker_interface_yields_to_user_owned_type_names() {
    let taken = with_type(colliding_api(), "ISchemaError", false);
    for files in [csharp(&taken), java(&taken)] {
        let source = joined(&files);
        assert!(source.contains("interface ISchemaErrorFamily"), "{source}");
        assert!(!source.contains("interface ISchemaError {"), "{source}");
    }
    let php = gen_php_error_types(&with_type(colliding_api(), "SchemaErrorInterface", false), "Demo", None).unwrap();
    assert!(php.contains("SchemaErrorInterfaceFamily"), "{php}");
    let trait_interface = gen_php_error_types(&with_type(colliding_api(), "SchemaError", true), "Demo", None).unwrap();
    assert!(
        trait_interface.contains("SchemaErrorInterfaceFamily"),
        "{trait_interface}"
    );
}
