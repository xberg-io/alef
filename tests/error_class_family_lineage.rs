//! Shared variant classes whose families inherit introspection methods through `exception_base`.
//!
//! `ChildError` inherits `status_code` from `ParentError`; the unrelated `OtherError` declares
//! `status_code` and `error_type` itself; `ChildError` and `OtherError` share `Shared`. The shared
//! class extends `ChildErrorException`, which already inherits `status_code`, so it must not
//! redeclare it, and it must report `error_type` only when `OtherError` raised it.

use alef::backends::csharp::CsharpBackend;
use alef::backends::java::JavaBackend;
use alef::codegen::error_gen::{ExceptionNaming, shared_exception_classes, shared_introspection};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use std::process::Command;

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.into(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn error(name: &str, base: Option<&str>, variants: &[(&str, u32)], methods: Vec<MethodDef>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        variants: variants
            .iter()
            .map(|(variant, code)| ErrorVariant {
                name: (*variant).into(),
                error_code: Some(*code),
                message_template: Some(variant.to_lowercase()),
                is_unit: true,
                ..Default::default()
            })
            .collect(),
        exception_base: base.map(str::to_string),
        original_rust_path: String::new(),
        doc: String::new(),
        methods,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn status_code() -> MethodDef {
    method("status_code", TypeRef::Primitive(PrimitiveType::U16))
}

fn lineage_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        errors: vec![
            error("ChildError", Some("ParentError"), &[("Shared", 111)], vec![]),
            error(
                "OtherError",
                None,
                &[("Shared", 112)],
                vec![status_code(), method("error_type", TypeRef::String)],
            ),
            error("ParentError", None, &[("Base", 110)], vec![status_code()]),
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

#[test]
fn shared_introspection_includes_methods_a_declaring_family_inherits() {
    let api = lineage_api();
    let shared = shared_exception_classes(&api.errors, ExceptionNaming::Nominal);
    let members = shared.get("SharedException").expect("Shared is shared by two families");
    let union = shared_introspection(&api.errors, "SharedException", members).unwrap();
    let summary: Vec<(&str, bool)> = union
        .iter()
        .map(|(method, nullable)| (method.name.as_str(), *nullable))
        .collect();
    assert_eq!(summary, [("status_code", false), ("error_type", true)]);
}

#[test]
fn an_inherited_method_typed_differently_by_a_sharing_family_is_rejected() {
    let mut api = lineage_api();
    api.errors[1].methods[0] = method("status_code", TypeRef::Primitive(PrimitiveType::U32));
    let shared = shared_exception_classes(&api.errors, ExceptionNaming::Nominal);
    let members = shared.get("SharedException").unwrap();
    let message = shared_introspection(&api.errors, "SharedException", members)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("status_code") && message.contains("OtherError"),
        "{message}"
    );
}

fn java_files() -> Vec<GeneratedFile> {
    let config = config(
        "\"java\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\nerror_style = \"last_error\"\n\n[crates.java]\npackage = \"demo\"\n",
    );
    JavaBackend
        .generate_bindings(&lineage_api(), &config)
        .expect("java bindings")
}

/// Compiles the generated Java exceptions with every dispatch arm as a constructor call.
#[test]
fn generated_java_shared_class_compiles_when_its_base_inherits_an_introspection_method() {
    let files = java_files();
    let shared = file(&files, "SharedException.java");
    assert!(
        !shared.contains("getStatusCode"),
        "inherited from ParentErrorException:\n{shared}"
    );
    assert!(shared.contains("public String getErrorType()"), "{shared}");
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
    let arms: String = files
        .iter()
        .flat_map(|file| file.content.lines())
        .filter(|line| line.trim_start().starts_with("case 11"))
        .map(|line| format!("      {}\n", line.trim()))
        .collect();
    assert_eq!(arms.matches("case 11").count(), 3, "{arms}");
    let probe = package.join("LineageProbe.java");
    std::fs::write(
        &probe,
        format!(
            "package demo;\npublic class LineageProbe {{\n  static Exception build(int code, String msg, int statusCodeNative, \
             String errorTypeNative) {{\n    return switch (code) {{\n{arms}      default -> null;\n    }};\n  }}\n  \
             public static void main(String[] args) {{\n    SharedException other = (SharedException) build(112, \"m\", 7, \"x\");\n    \
             SharedException child = (SharedException) build(111, \"m\", 7, \"x\");\n    \
             if (!\"x\".equals(other.getErrorType())) throw new AssertionError(\"error type lost\");\n    \
             if (child.getErrorType() != null) throw new AssertionError(\"error type invented\");\n    \
             if (!(other instanceof IOtherError) || !(other instanceof IParentError)) throw new AssertionError(\"family lost\");\n    \
             System.out.println(\"LINEAGE=3\");\n  }}\n}}\n"
        ),
    )
    .unwrap();
    sources.push(probe);
    let compile = Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "demo.LineageProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "LINEAGE=3");
}

fn csharp_files() -> Vec<GeneratedFile> {
    let config = config(
        "\"csharp\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\nerror_style = \"last_error\"\n\n[crates.csharp]\nnamespace = \"Demo\"\n",
    );
    CsharpBackend
        .generate_bindings(&lineage_api(), &config)
        .expect("csharp bindings")
}

#[test]
fn csharp_shared_class_applies_a_value_only_for_the_families_that_report_it() {
    let files = csharp_files();
    let shared = file(&files, "SharedException.cs");
    assert!(
        !shared.contains("StatusCode"),
        "inherited from ParentErrorException:\n{shared}"
    );
    assert!(shared.contains("internal new void ApplyLastError()"), "{shared}");
    assert!(shared.contains("if (Code == 112)"), "{shared}");
}

/// Compiles the generated C# exceptions as warnings-as-errors and drives `ApplyLastError` the way
/// `FromLastError` does, for a code of each family that raises the shared class.
#[test]
fn generated_csharp_shared_class_compiles_and_gates_values_on_the_raising_family() {
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
            || generated.content.contains("public interface I")
        {
            std::fs::write(directory.path().join(name), generated.content).unwrap();
            written += 1;
        }
    }
    assert_eq!(written, 8, "three bases, two variant classes and three interfaces");
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
        private static SharedException Raise(int code)
        {
            Exception exception = new SharedException("shared") { Code = code };
            if (exception is ParentErrorException family) family.ApplyLastError();
            if (exception is SharedException own) own.ApplyLastError();
            return (SharedException)exception;
        }

        private static void Main()
        {
            NativeMethods.StatusCode = 7;
            NativeMethods.ErrorType = Marshal.StringToCoTaskMemUTF8("other");
            var other = Raise(112);
            if (other.ErrorType != "other" || other.StatusCode != 7 || !(other is IOtherError))
                throw new Exception($"OtherError lost a value: {other.ErrorType} {other.StatusCode}");
            var child = Raise(111);
            if (child.ErrorType != null || !(child is IParentError))
                throw new Exception($"ChildError reported a value it lacks: {child.ErrorType}");
            Console.WriteLine("LINEAGE=2");
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
<TreatWarningsAsErrors>true</TreatWarningsAsErrors><NuGetAudit>false</NuGetAudit></PropertyGroup></Project>"#,
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
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "LINEAGE=2");
}

#[test]
fn csharp_shared_class_without_stable_codes_documents_the_native_default() {
    let mut api = lineage_api();
    api.errors[0].variants[0].error_code = None;
    let config = config(
        "\"csharp\", \"ffi\"",
        "[crates.ffi]\nprefix = \"demo\"\nerror_style = \"last_error\"\n\n[crates.csharp]\nnamespace = \"Demo\"\n",
    );
    let files = CsharpBackend.generate_bindings(&api, &config).expect("csharp bindings");
    let shared = file(&files, "SharedException.cs");
    assert!(!shared.contains("if (Code =="), "{shared}");
    assert!(
        shared.contains("the native layer's default (0, empty or <c>null</c>)"),
        "{shared}"
    );
}
