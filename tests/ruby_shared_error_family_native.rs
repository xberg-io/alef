//! Compile actual Magnus registrations/converters and exercise Ruby rescue dispatch.

#[path = "support/mise_task.rs"]
mod mise_task;

use alef::codegen::error_gen::{gen_magnus_error_class_registrations, gen_magnus_error_converter};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.into(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn family(name: &str, parent: Option<&str>, variants: &[&str], methods: Vec<MethodDef>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("core_types::{name}"),
        original_rust_path: String::new(),
        exception_base: parent.map(str::to_string),
        variants: variants
            .iter()
            .map(|name| ErrorVariant {
                name: (*name).into(),
                is_unit: true,
                ..Default::default()
            })
            .collect(),
        doc: String::new(),
        methods,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        errors: vec![
            family("GraphQLError", None, &["Shared", "GraphOnly"], vec![status_code()]),
            family(
                "SchemaError",
                Some("RootError"),
                &["Shared", "SchemaOnly"],
                vec![status_code(), method("error_type", TypeRef::String)],
            ),
            family("RootError", None, &["RootOnly"], vec![]),
        ],
        ..Default::default()
    }
}

fn status_code() -> MethodDef {
    method("status_code", TypeRef::Primitive(PrimitiveType::U16))
}

fn native_source() -> String {
    let api = api();
    let registrations = gen_magnus_error_class_registrations(&api).expect("actual Magnus registrations");
    assert_eq!(
        registrations
            .iter()
            .filter(|line| line.contains("define_error(\"SharedError\""))
            .count(),
        1,
        "exercise one globally shared class"
    );
    let converters = api.errors[..2]
        .iter()
        .map(|error| gen_magnus_error_converter(error, "core_types", "RubyFamilyNative"))
        .collect::<Vec<_>>();
    assert_eq!(converters.len(), 2);
    let template = include_str!("fixtures/ruby_shared_family/native.rs.txt");
    assert_eq!(template.matches("// CONVERTERS").count(), 1);
    assert_eq!(template.matches("// REGISTRATIONS").count(), 1);
    let source = template
        .replace("// CONVERTERS", &converters.join("\n"))
        .replace("// REGISTRATIONS", &registrations.join("\n"));
    syn::parse_file(&source).expect("actual generated native fixture parses");
    source
}

fn write_fixture(directory: &Path) {
    std::fs::create_dir(directory.join("src")).unwrap();
    let manifest = include_str!("fixtures/ruby_error_matcher/Cargo.toml.txt");
    assert_eq!(manifest.matches("ruby_error_matcher_fixture").count(), 1);
    std::fs::write(
        directory.join("Cargo.toml"),
        manifest.replace("ruby_error_matcher_fixture", "ruby_family_native_fixture"),
    )
    .unwrap();
    std::fs::write(directory.join("src/lib.rs"), native_source()).unwrap();
    std::fs::write(
        directory.join("check.rb"),
        include_str!("fixtures/ruby_shared_family/check.rb"),
    )
    .unwrap();
    std::fs::write(
        directory.join("mise.toml"),
        "[tasks.build]\nrun = 'cargo build --quiet'\n",
    )
    .unwrap();
    if cfg!(target_os = "macos") {
        std::fs::write(
            directory.join("build.rs"),
            "fn main() { println!(\"cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup\"); }\n",
        )
        .unwrap();
    }
}

fn ruby_configuration(ruby: &Path) -> serde_json::Value {
    let output = Command::new(ruby)
        .args([
            "-rjson",
            "-rrbconfig",
            "-e",
            "print({arch:RbConfig::CONFIG.fetch('arch'),extension:RbConfig::CONFIG.fetch('DLEXT')}.to_json)",
        ])
        .output()
        .expect("selected Ruby is required, not skipped");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("actual Ruby architecture and extension suffix")
}

fn build_extension(directory: &Path, ruby: &Path, config: &serde_json::Value) -> PathBuf {
    let target = match config["arch"].as_str().unwrap() {
        "x64-mingw-ucrt" | "x64-mingw32" | "x86_64-mingw32" => Some("x86_64-pc-windows-gnu"),
        _ => None,
    };
    let mut command = mise_task::mise_run_without_lock(directory, "build");
    command
        .env("RUBY", ruby)
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .env("CARGO_BUILD_JOBS", "2")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env_remove("CARGO_BUILD_TARGET");
    if let Some(target) = target {
        command.env("CARGO_BUILD_TARGET", target);
    }
    let output = command.output().expect("build isolated real Magnus fixture");
    assert_success(&output);
    install_extension(directory, target, config["extension"].as_str().unwrap())
}

fn install_extension(directory: &Path, target: Option<&str>, suffix: &str) -> PathBuf {
    assert!(!suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_alphanumeric()));
    let mut artifacts = directory.join("target");
    if let Some(target) = target {
        artifacts.push(target);
    }
    let library = if cfg!(target_os = "macos") {
        "libruby_family_native_fixture.dylib"
    } else if cfg!(windows) {
        "ruby_family_native_fixture.dll"
    } else {
        "libruby_family_native_fixture.so"
    };
    let destination = directory.join(format!("ruby_family_native_fixture.{suffix}"));
    std::fs::copy(artifacts.join("debug").join(library), &destination).expect("load the freshly built fixture");
    destination
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn should_rescue_real_shared_magnus_errors_through_both_families_and_ancestors() {
    let ruby = which::which("ruby").expect("managed Ruby and its development headers are required");
    let directory = tempfile::tempdir().expect("owned isolated native target, removed with the fixture");
    write_fixture(directory.path());
    let config = ruby_configuration(&ruby);
    let extension = build_extension(directory.path(), &ruby, &config);
    let invoke = |poison: bool| {
        let mut command = Command::new(&ruby);
        command.arg(directory.path().join("check.rb")).arg(&extension);
        if poison {
            command.arg("--poison-family");
        }
        command.output().expect("execute real Ruby rescue dispatch")
    };
    let positive = invoke(false);
    assert_success(&positive);
    assert_eq!(
        String::from_utf8(positive.stdout).unwrap(),
        "PASS: 36 checks; 2 converters; 7 negative oracles\n"
    );
    let negative = invoke(true);
    assert!(
        !negative.status.success(),
        "removing secondary family rescue must break the behavioral oracle"
    );
    assert!(
        String::from_utf8_lossy(&negative.stderr).contains("CHECK_FAILED secondary_graph_schema"),
        "{}",
        String::from_utf8_lossy(&negative.stderr)
    );
}
