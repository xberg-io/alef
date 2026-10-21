use alef::codegen::error_gen::{gen_php_error_converter, gen_php_error_types};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
#[ignore = "requires PHP development headers and the PHP executable"]
fn should_compile_and_throw_shared_errors_with_cfg_features_disabled_and_enabled() {
    let mut fields = vec![
        FieldDef {
            name: "reason".into(),
            ty: TypeRef::String,
            ..Default::default()
        },
        FieldDef {
            name: "kind".into(),
            ty: TypeRef::Primitive(PrimitiveType::U16),
            ..Default::default()
        },
        FieldDef {
            name: "status".into(),
            ty: TypeRef::Primitive(PrimitiveType::U16),
            cfg: Some("feature = \"http\"".into()),
            ..Default::default()
        },
    ];
    let first = error("ConversionError", "CoreError", fields.clone());
    fields.swap(0, 1);
    let second = error("SecondaryError", "SecondaryCoreError", fields);
    let api = ApiSurface {
        errors: vec![first.clone(), second.clone()],
        ..Default::default()
    };
    let source = format!(
        "{}\n{}\n{}\n{}",
        fixture_source(),
        gen_php_error_types(&api, "Sample").unwrap(),
        gen_php_error_converter(&first, "sample"),
        gen_php_error_converter(&second, "sample")
    );
    let fixture = tempfile::tempdir().unwrap();
    write_fixture(fixture.path(), &source);
    for enabled in [false, true] {
        let library = build_extension(fixture.path(), enabled);
        let php = format!(
            "if (http_enabled() !== {enabled}) throw new Exception('incorrect feature mode');\n{}",
            php_probe()
        );
        let runtime = Command::new("php")
            .args(["-d"])
            .arg(format!("extension={}", library.display()))
            .args(["-r", &php])
            .output()
            .unwrap();
        assert!(
            runtime.status.success(),
            "http={enabled}: stdout={} stderr={}",
            String::from_utf8_lossy(&runtime.stdout),
            String::from_utf8_lossy(&runtime.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&runtime.stdout), "2 cases passed\n");
    }
}

fn error(name: &str, rust_path: &str, fields: Vec<FieldDef>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: rust_path.into(),
        variants: vec![ErrorVariant {
            name: "Rejected".into(),
            fields,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn write_fixture(root: &Path, source: &str) {
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir(root.join(".cargo")).unwrap();
    std::fs::write(root.join("src/lib.rs"), source).unwrap();
    std::fs::write(
        root.join(".cargo/config.toml"),
        "[target.'cfg(target_os = \"macos\")']\nrustflags = [\"-C\", \"link-arg=-Wl,-undefined,dynamic_lookup\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "alef-php-cfg-payload-probe"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[features]
http = []
[dependencies]
ext-php-rs = "=0.16.3"
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Taskfile.yml"),
        "version: '3'\ntasks:\n  build:\n    cmds:\n      - cargo build --offline {{.CLI_ARGS}}\n",
    )
    .unwrap();
}

fn build_extension(root: &Path, enabled: bool) -> PathBuf {
    let target = std::env::var_os("ALEF_PHP_CFG_NATIVE_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let mut command = Command::new("task");
    command.arg("--dir").arg(root).arg("build");
    if enabled {
        command.args(["--", "--features", "http"]);
    }
    let build = command.env("CARGO_TARGET_DIR", &target).output().unwrap();
    assert!(
        build.status.success(),
        "http={enabled}: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let library = target.join("debug").join(format!(
        "{}alef_php_cfg_payload_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file());
    library
}

fn fixture_source() -> &'static str {
    r#"
use ext_php_rs::prelude::*;
enum CoreError { Rejected { reason: &'static str, kind: u16, #[cfg(feature = "http")] status: u16 } }
enum SecondaryCoreError { Rejected { kind: u16, reason: &'static str, #[cfg(feature = "http")] status: u16 } }
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("primary failure") }
}
impl std::fmt::Display for SecondaryCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("secondary failure") }
}
#[php_function]
fn fail_primary() -> Result<(), ext_php_rs::exception::PhpException> {
    Err(conversion_error_to_php_err(CoreError::Rejected { reason: "primary", kind: 0, #[cfg(feature = "http")] status: 429 }))
}
#[php_function]
fn fail_secondary() -> Result<(), ext_php_rs::exception::PhpException> {
    Err(secondary_error_to_php_err(SecondaryCoreError::Rejected { kind: 403, reason: "secondary", #[cfg(feature = "http")] status: 503 }))
}
#[php_function]
fn http_enabled() -> bool { cfg!(feature = "http") }
#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module.class::<PhpConversionError>().class::<PhpSecondaryError>().class::<PhpRejectedError>()
        .function(wrap_function!(fail_primary)).function(wrap_function!(fail_secondary)).function(wrap_function!(http_enabled))
}
"#
}

fn php_probe() -> &'static str {
    r#"
foreach ([['fail_primary', 'primary', 0], ['fail_secondary', 'secondary', 403]] as [$call, $reason, $kind]) {
    try { $call(); throw new Exception('missing failure'); }
    catch (Sample\RejectedError $error) {
        if (!($error instanceof Sample\ConversionError) || $error->reason !== $reason || $error->kind !== $kind
            || $error->getMessage() !== "$reason failure" || property_exists($error, 'status')) {
            throw new Exception('incorrect payload: ' . json_encode(get_object_vars($error)));
        }
    }
}
echo "2 cases passed\n";
"#
}
