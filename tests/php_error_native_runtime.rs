use alef::codegen::error_gen::{gen_php_error_converter, gen_php_error_types};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};
use std::process::Command;

#[test]
#[ignore = "requires PHP development headers and the PHP executable"]
fn should_throw_generated_php_exception_with_message_and_payload() {
    let error = probe_error();
    let api = ApiSurface {
        errors: vec![error.clone()],
        ..Default::default()
    };
    let source = format!(
        "{}\n{}\n{}",
        fixture_source(),
        gen_php_error_types(&api, "Sample").unwrap(),
        gen_php_error_converter(&error, "sample")
    );
    let fixture = tempfile::tempdir().unwrap();
    let library = build_extension(fixture.path(), &source);
    let runtime = Command::new("php")
        .arg("-d")
        .arg(format!("extension={}", library.display()))
        .arg("-r")
        .arg(php_probe())
        .output()
        .unwrap();
    assert!(
        runtime.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&runtime.stdout), "3 cases passed\n");
}

fn probe_error() -> ErrorDef {
    ErrorDef {
        name: "ConversionError".into(),
        rust_path: "CoreError".into(),
        variants: vec![ErrorVariant {
            name: "Rejected".into(),
            fields: vec![
                FieldDef {
                    name: "vendor".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                FieldDef {
                    name: "status".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U16))),
                    ..Default::default()
                },
                FieldDef {
                    name: "body".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    ..Default::default()
                },
            ],
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

fn build_extension(fixture: &std::path::Path, source: &str) -> std::path::PathBuf {
    std::fs::create_dir(fixture.join("src")).unwrap();
    std::fs::create_dir(fixture.join(".cargo")).unwrap();
    std::fs::write(
        fixture.join(".cargo/config.toml"),
        "[target.'cfg(target_os = \"macos\")']\nrustflags = [\"-C\", \"link-arg=-Wl,-undefined,dynamic_lookup\"]\n",
    )
    .unwrap();
    std::fs::write(fixture.join("src/lib.rs"), source).unwrap();
    std::fs::write(
        fixture.join("Cargo.toml"),
        r#"[package]
name = "alef-php-error-probe"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
ext-php-rs = "=0.16.3"
"#,
    )
    .unwrap();
    std::fs::write(
        fixture.join("Taskfile.yml"),
        "version: '3'\ntasks:\n  build:\n    cmds:\n      - cargo build --offline\n",
    )
    .unwrap();
    let target = std::env::var_os("ALEF_PHP_NATIVE_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.join("target"));
    let build = Command::new("task")
        .args(["--dir"])
        .arg(fixture)
        .arg("build")
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let library = target.join("debug").join(format!(
        "{}alef_php_error_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "{}", library.display());
    library
}

fn fixture_source() -> &'static str {
    r#"
use ext_php_rs::prelude::*;
enum CoreError { Rejected { vendor: &'static str, status: Option<u16>, body: Option<&'static str> } }
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("blocked by cloudflare") }
}
#[php_function]
fn fail(case: i64) -> Result<(), ext_php_rs::exception::PhpException> {
    Err(conversion_error_to_php_err(CoreError::Rejected {
        vendor: "cloudflare", status: match case { 0 => None, 1 => Some(0), _ => Some(403) },
        body: match case { 0 => None, 1 => Some(""), _ => Some("challenge") },
    }))
}
#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module.class::<PhpConversionError>().class::<PhpRejectedError>().function(wrap_function!(fail))
}
"#
}

fn php_probe() -> &'static str {
    r#"
foreach ([[null, null], [0, ''], [403, 'challenge']] as $case => [$status, $body]) {
    try { fail($case); throw new Exception('missing failure'); }
    catch (Sample\RejectedError $error) {
        if (!($error instanceof Sample\ConversionError) || !($error instanceof Exception)
            || $error->getMessage() !== 'blocked by cloudflare' || $error->vendor !== 'cloudflare'
            || $error->status !== $status || $error->body !== $body
            || $error->getCode() !== 0 || $error->getFile() !== 'Command line code'
            || $error->getLine() !== 3 || count($error->getTrace()) < 1) {
            throw new Exception('exception details changed: ' . json_encode([
                get_class($error), $error->getMessage(), $error->vendor, $error->status, $error->body,
                $error->getCode(), $error->getFile(), $error->getLine(), $error->getTrace(),
            ]));
        }
    }
}
echo "3 cases passed\n";
"#
}
