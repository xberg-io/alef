//! A variant class shared by unrelated error families carries the introspection values of every
//! family that raises it: the union of their methods, filled by each family's converter from its
//! own error. Java and C# leave the values a family lacks `null`; PHP fills the type's default,
//! since its getters must stay compatible with the non-nullable ones the parent and root declare.

use alef::backends::csharp::CsharpBackend;
use alef::backends::java::JavaBackend;
use alef::codegen::error_gen::{
    gen_magnus_error_class_registrations, gen_php_error_converter_with_errors, gen_php_error_stubs,
    gen_php_error_types, php_error_class_names,
};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use std::process::Command;

#[path = "support/php_native.rs"]
mod php_native;

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.into(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn variant(name: &str, code: u32) -> ErrorVariant {
    ErrorVariant {
        name: name.into(),
        error_code: Some(code),
        message_template: Some(name.to_lowercase()),
        is_unit: true,
        ..Default::default()
    }
}

fn error(name: &str, variants: Vec<ErrorVariant>, methods: Vec<MethodDef>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        variants,
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// `GraphQLError` reports a `status_code`, `SchemaError` an `error_type`, and both declare
/// `ComplexityLimitExceeded`.
fn introspection_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        errors: vec![
            error(
                "GraphQLError",
                vec![variant("ComplexityLimitExceeded", 100), variant("Parse", 101)],
                vec![method("status_code", TypeRef::Primitive(PrimitiveType::U16))],
            ),
            error(
                "SchemaError",
                vec![variant("ComplexityLimitExceeded", 102), variant("Invalid", 103)],
                vec![method("error_type", TypeRef::String)],
            ),
        ],
        ..Default::default()
    }
}

fn config(languages: &str, sections: &str) -> ResolvedCrateConfig {
    let toml = format!(
        "[workspace]\nlanguages = [{languages}]\n\n[[crates]]\nname = \"demo\"\nsources = [\"src/lib.rs\"]\n\n{sections}"
    );
    let config: NewAlefConfig = toml::from_str(&toml).expect("valid config");
    config.resolve().expect("resolved config").remove(0)
}

fn file<'a>(files: &'a [GeneratedFile], name: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|file_name| file_name == name))
        .unwrap_or_else(|| panic!("missing {name}"))
        .content
}

/// The Rust struct item `name` declares in `source`, up to its closing brace.
fn struct_item<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("missing struct {name}"));
    let end = source[start..].find("\n}").unwrap() + start + 2;
    &source[start..end]
}

#[test]
fn php_shared_class_carries_every_family_introspection() {
    let api = introspection_api();
    let types = gen_php_error_types(&api, "Demo", None).unwrap();
    assert_eq!(
        struct_item(&types, "PhpComplexityLimitExceededError"),
        "pub struct PhpComplexityLimitExceededError {\n    info_status_code: u16,\n    \
         info_error_type: String,\n}"
    );
    assert_eq!(
        struct_item(&types, "PhpParseError"),
        "pub struct PhpParseError {\n    info_status_code: u16,\n}"
    );
    let stubs = gen_php_error_stubs(&api, None).unwrap();
    let shared = &stubs[stubs.find("class ComplexityLimitExceededError").unwrap()..];
    let shared = &shared[..shared.find("\n}").unwrap()];
    assert!(shared.contains("public function statusCode(): int"), "{shared}");
    assert!(shared.contains("public function errorType(): string"), "{shared}");
    assert!(
        shared.contains(
            "/** `error_type` of the error. The type's default (0, '' or false) when the family that \
             raised it has no such value. */"
        ),
        "{shared}"
    );
    let parse = &stubs[stubs.find("class ParseError").unwrap()..];
    let parse = &parse[..parse.find("\n}").unwrap()];
    assert!(
        !parse.contains("default"),
        "a class one family raises has every value:\n{parse}"
    );
}

#[test]
fn php_converters_fill_their_own_introspection_and_default_the_other_family() {
    let api = introspection_api();
    let graphql = gen_php_error_converter_with_errors(&api.errors[0], &api.errors, "demo");
    assert!(
        graphql.contains(
            "PhpComplexityLimitExceededError {\n            info_status_code: e.status_code(),\n            \
             info_error_type: Default::default(),\n        }"
        ),
        "{graphql}"
    );
    assert!(
        graphql.contains("PhpParseError {\n            info_status_code: e.status_code(),\n        }"),
        "{graphql}"
    );
    let schema = gen_php_error_converter_with_errors(&api.errors[1], &api.errors, "demo");
    assert!(
        schema.contains(
            "PhpComplexityLimitExceededError {\n            info_status_code: Default::default(),\n            \
             info_error_type: e.error_type().to_string(),\n        }"
        ),
        "{schema}"
    );
}

#[test]
fn php_rejects_one_introspection_method_typed_differently_by_two_sharing_families() {
    let mut api = introspection_api();
    api.errors[1].methods = vec![method("status_code", TypeRef::Primitive(PrimitiveType::U32))];
    let message = gen_php_error_types(&api, "Demo", None).unwrap_err().to_string();
    for expected in [
        "status_code",
        "ComplexityLimitExceededError",
        "GraphQLError",
        "SchemaError",
    ] {
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn ruby_shared_class_reads_every_family_introspection() {
    let lines = gen_magnus_error_class_registrations(&introspection_api()).unwrap();
    for attribute in ["status_code", "error_type"] {
        assert!(
            lines.contains(&format!(
                "    complexity_limit_exceeded_error_class.define_attr(\"{attribute}\", magnus::Attr::Read)?;"
            )),
            "{lines:#?}"
        );
    }
}

fn java_files() -> Vec<GeneratedFile> {
    let config = config(
        "\"java\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\nerror_style = \"last_error\"\n\n[crates.java]\npackage = \"demo\"\n",
    );
    JavaBackend
        .generate_bindings(&introspection_api(), &config)
        .expect("java bindings")
}

/// The `case <code> -> ...;` arms of the generated last-error dispatch.
fn dispatch_arms(files: &[GeneratedFile]) -> Vec<String> {
    files
        .iter()
        .flat_map(|file| file.content.lines())
        .filter(|line| line.trim_start().starts_with("case 10"))
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn java_dispatch_builds_the_shared_class_with_both_family_introspection() {
    let files = java_files();
    let arms = dispatch_arms(&files);
    let shared = file(&files, "ComplexityLimitExceededException.java");
    assert!(
        shared.contains(
            "public static ComplexityLimitExceededException withIntrospection(final String message, final int statusCode, \
             final String errorType) {"
        ),
        "{shared}"
    );
    assert!(
        shared.contains("public String getErrorType() { return this.errorType; }"),
        "{shared}"
    );
    for code in [100, 102] {
        let arm = arms
            .iter()
            .find(|arm| arm.starts_with(&format!("case {code} ->")))
            .unwrap_or_else(|| panic!("no arm for {code}: {arms:#?}"));
        assert_eq!(
            arm.matches(',').count(),
            2,
            "code {code} must pass message, status code and error type: {arm}"
        );
    }
}

/// Compiles the generated exceptions with every dispatch arm as a constructor call.
#[test]
fn generated_java_dispatch_arms_compile_against_the_shared_class() {
    let files = java_files();
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("demo");
    std::fs::create_dir(&package).unwrap();
    let mut sources = Vec::new();
    for generated in &files {
        let name = generated.path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("Exception.java") || (name.starts_with('I') && name.ends_with("Error.java")) {
            let path = package.join(&name);
            std::fs::write(&path, &generated.content).unwrap();
            sources.push(path);
        }
    }
    let arms = dispatch_arms(&files);
    assert_eq!(arms.len(), 4, "{arms:#?}");
    let cases = arms.iter().map(|arm| format!("      {arm}\n")).collect::<String>();
    let probe = package.join("ArmProbe.java");
    std::fs::write(
        &probe,
        format!(
            "package demo;\npublic class ArmProbe {{\n  static Exception build(int code, String msg, int statusCodeNative, \
             String errorTypeNative) {{\n    return switch (code) {{\n{cases}      default -> null;\n    }};\n  }}\n  \
             public static void main(String[] args) {{\n    ComplexityLimitExceededException shared = \
             (ComplexityLimitExceededException) build(102, \"m\", 0, \"schema\");\n    \
             if (!\"schema\".equals(shared.getErrorType())) throw new AssertionError(\"error type lost\");\n    \
             System.out.println(\"ARMS=4\");\n  }}\n}}\n"
        ),
    )
    .unwrap();
    sources.push(probe);
    let compile = Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "demo.ArmProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ARMS=4");
}

fn csharp_files() -> Vec<GeneratedFile> {
    let config = config(
        "\"csharp\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\nerror_style = \"last_error\"\n\n[crates.csharp]\nnamespace = \"Demo\"\n",
    );
    CsharpBackend
        .generate_bindings(&introspection_api(), &config)
        .expect("csharp bindings")
}

#[test]
fn csharp_shared_class_declares_the_other_family_introspection() {
    let files = csharp_files();
    let shared = file(&files, "ComplexityLimitExceededException.cs");
    assert!(
        shared.contains("public string? ErrorType { get; internal set; }"),
        "{shared}"
    );
    assert!(
        !shared.contains("StatusCode"),
        "the base family already declares it:\n{shared}"
    );
    assert!(shared.contains("internal new void ApplyLastError()"), "{shared}");
    let dispatcher = files
        .iter()
        .find(|file| file.content.contains("internal static Exception FromLastError"))
        .expect("dispatcher");
    assert!(
        dispatcher
            .content
            .contains("if (exception is ComplexityLimitExceededException nativeError"),
        "{}",
        dispatcher.content
    );
}

/// Compiles the generated C# exceptions with a probe that reads the shared class's properties.
#[test]
fn generated_csharp_shared_class_compiles_with_both_family_introspection() {
    if Command::new("dotnet").arg("--version").output().is_err() {
        assert!(std::env::var_os("ALEF_REQUIRE_DOTNET").is_none(), "dotnet is required");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut written = 0;
    for generated in csharp_files() {
        let name = generated.path.file_name().unwrap().to_string_lossy().into_owned();
        let is_error = generated.content.contains("public class ") && generated.content.contains("Exception : ");
        if (is_error && !generated.content.contains("FromLastError"))
            || generated.content.contains("public interface IGraphQLError")
            || generated.content.contains("public interface ISchemaError")
        {
            std::fs::write(directory.path().join(name), generated.content).unwrap();
            written += 1;
        }
    }
    assert_eq!(written, 7, "two bases, three variant classes and two interfaces");
    std::fs::write(
        directory.path().join("Program.cs"),
        r#"using System;
using System.Runtime.InteropServices;
namespace Demo
{
    public class DemoException : Exception
    {
        public int Code { get; internal set; }
        public DemoException(string message) : base(message) { }
        public DemoException(string message, Exception inner) : base(message, inner) { }
    }
    internal static class NativeMethods
    {
        internal static ushort StatusCode;
        internal static IntPtr ErrorType;
        internal static ushort LastErrorStatusCode() => StatusCode;
        internal static IntPtr LastErrorErrorType() => ErrorType;
    }
    internal static class Program
    {
        private static void Main()
        {
            NativeMethods.StatusCode = 7;
            NativeMethods.ErrorType = Marshal.StringToCoTaskMemUTF8("schema");
            Exception exception = new ComplexityLimitExceededException("limit") { Code = 102 };
            // The dispatcher applies the base family's getters, then the shared class's own.
            if (exception is GraphQLErrorException family) family.ApplyLastError();
            if (exception is ComplexityLimitExceededException own) own.ApplyLastError();
            var shared = (ComplexityLimitExceededException)exception;
            if (!(shared is ISchemaError) || shared.ErrorType != "schema" || shared.StatusCode != 7)
                throw new Exception($"shared class lost an introspection value: {shared.StatusCode} {shared.ErrorType}");
            if (new ComplexityLimitExceededException("unset").ErrorType != null)
                throw new Exception("unset introspection value is not null");
            Console.WriteLine("SHARED=1");
        }
    }
}
"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("Probe.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType>
<TargetFramework>net8.0</TargetFramework><RollForward>LatestMajor</RollForward><Nullable>enable</Nullable>
<NuGetAudit>false</NuGetAudit></PropertyGroup></Project>"#,
    )
    .unwrap();
    let output = Command::new("dotnet")
        .args(["run", "--project", "Probe.csproj", "--verbosity", "quiet"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "SHARED=1");
}

/// Builds the generated ext-php-rs classes and reads each family's introspection back.
#[test]
#[ignore = "requires PHP development headers and the PHP executable"]
fn generated_php_shared_class_compiles_and_reports_each_family_introspection() {
    let api = introspection_api();
    let source = format!(
        r#"
use ext_php_rs::prelude::*;
mod demo {{
    pub enum GraphQLError {{ ComplexityLimitExceeded, Parse }}
    pub enum SchemaError {{ ComplexityLimitExceeded, Invalid }}
    impl GraphQLError {{ pub fn status_code(&self) -> u16 {{ 429 }} }}
    impl SchemaError {{ pub fn error_type(&self) -> &'static str {{ "schema" }} }}
    impl std::fmt::Display for GraphQLError {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str("graphql") }} }}
    impl std::fmt::Display for SchemaError {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str("schema") }} }}
}}
#[php_function]
fn fail_graphql() -> Result<(), ext_php_rs::exception::PhpException> {{ Err(graph_ql_error_to_php_err(demo::GraphQLError::ComplexityLimitExceeded)) }}
#[php_function]
fn fail_schema() -> Result<(), ext_php_rs::exception::PhpException> {{ Err(schema_error_to_php_err(demo::SchemaError::ComplexityLimitExceeded)) }}
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
foreach ([['fail_graphql', 429, ''], ['fail_schema', 0, 'schema']] as [$call, $status, $type]) {
    try { $call(); throw new Exception('missing failure'); }
    catch (Demo\ComplexityLimitExceededError $error) {
        if ($error->statusCode() !== $status || $error->errorType() !== $type) {
            throw new Exception("$call: " . var_export([$error->statusCode(), $error->errorType()], true));
        }
    }
}
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
