use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::DefaultValue;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
#[ignore = "requires PHP development headers and PHP"]
fn should_preserve_legacy_php_constructor_without_serde() {
    run_constructor_probe(false);
}

#[test]
#[ignore = "requires PHP development headers and PHP"]
fn should_preserve_optional_php_constructor_defaults_with_serde() {
    run_constructor_probe(true);
}

fn run_constructor_probe(serde: bool) {
    let fixture = tempfile::tempdir().unwrap();
    let generated = generate(fixture.path(), serde);
    let source = format!(
        "{generated}\npub mod fixture {{ pub mod core {{ {} }} }}",
        core_source().replace("#[alef(legacy_constructor_fields = [\"url\"])]", "")
    );
    let library = build_extension(fixture.path(), &source);
    let script = format!(
        "$serde = {serde};\n{}\n{}",
        php_probe(),
        if serde {
            ""
        } else {
            r#"try { new Fixture\RequiredResponse(); throw new Exception('missing complex guard'); } catch (Exception $e) { if (!str_contains($e->getMessage(), 'requires complex params')) { throw $e; } }"#
        }
    );
    let output = Command::new("php")
        .arg("-d")
        .arg(format!("extension={}", library.display()))
        .arg("-r")
        .arg(script)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7 constructor cases passed\n");
}

fn generate(directory: &Path, serde: bool) -> String {
    let source = directory.join("core.rs");
    std::fs::write(&source, core_source()).unwrap();
    let mut api = alef::extract::extractor::extract(&[source.as_path()], "fixture", "1.0.0", None).unwrap();
    let response = api
        .types
        .iter_mut()
        .find(|typ| typ.name == "FalseDefaultResponse")
        .unwrap();
    response
        .fields
        .iter_mut()
        .find(|field| field.name == "retry_enabled")
        .unwrap()
        .typed_default = Some(DefaultValue::BoolLiteral(false));
    response
        .fields
        .iter_mut()
        .find(|field| field.name == "count")
        .unwrap()
        .typed_default = Some(DefaultValue::IntLiteral(0));
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=[\"php\"]\n[[crates]]\nname=\"fixture\"\nsources=[\"core.rs\"]\n")
            .unwrap();
    let mut config = raw.resolve().unwrap().remove(0);
    let detector = directory.join("detector");
    std::fs::create_dir_all(detector.join("src")).unwrap();
    std::fs::write(
        detector.join("Cargo.toml"),
        if serde {
            "[dependencies]\nserde=\"1\"\nserde_json=\"1\"\n"
        } else {
            "[dependencies]\n"
        },
    )
    .unwrap();
    config.output_paths.insert("php".into(), detector.join("src"));
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let files: Vec<_> = files
        .iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "lib.rs"))
        .collect();
    assert_eq!(files.len(), 1);
    files[0].content.clone()
}

fn build_extension(directory: &Path, source: &str) -> PathBuf {
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::create_dir_all(directory.join(".cargo")).unwrap();
    std::fs::write(
        directory.join(".cargo/config.toml"),
        "[target.'cfg(target_os = \"macos\")']\nrustflags=[\"-C\",\"link-arg=-Wl,-undefined,dynamic_lookup\"]\n",
    )
    .unwrap();
    std::fs::write(directory.join("src/lib.rs"), source).unwrap();
    std::fs::write(
        directory.join("Cargo.toml"),
        r#"[package]
name = "alef-php-constructor-probe"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
ext-php-rs = "=0.16.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("Taskfile.yml"),
        "version: '3'\ntasks:\n  build:\n    cmds:\n      - cargo build --offline\n",
    )
    .unwrap();
    let target = std::env::var_os("ALEF_PHP_NATIVE_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.join("target"));
    let output = Command::new("task")
        .arg("--dir")
        .arg(directory)
        .arg("build")
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    target.join("debug").join(format!(
        "{}alef_php_constructor_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
}

fn core_source() -> &'static str {
    r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BlockDetails { pub vendor: String }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyResponse { pub url: String }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct Response {
    pub url: String,
    #[serde(default)] pub block_details: Option<BlockDetails>,
    #[serde(default)] pub retry_enabled: Option<bool>,
    #[serde(default)] pub enforce: bool,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct FalseDefaultResponse { pub url: String, pub retry_enabled: Option<bool>, pub count: Option<u32> }
impl Default for FalseDefaultResponse {
    fn default() -> Self { Self { url: String::new(), retry_enabled: Some(false), count: Some(0) } }
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RequiredResponse { pub url: String, pub block_details: BlockDetails }
"#
}

fn php_probe() -> &'static str {
    r#"
$legacy = new Fixture\LegacyResponse('legacy');
if ($legacy->url !== 'legacy') { throw new Exception('legacy URL changed'); }
foreach ([['omitted', null], ['null', null], ['false', false], ['true', true]] as [$url, $retry]) {
    $args = [$url];
    if ($url !== 'omitted') {
        if ($serde) { $args[] = null; }
        $args[] = $retry;
        $args[] = $retry === true;
    }
    $response = new Fixture\Response(...$args);
    if ($response->url !== $url || $response->getBlockDetails() !== null
        || $response->retryEnabled !== $retry || $response->enforce !== ($retry === true)) {
        throw new Exception('optional constructor values changed');
    }
}
foreach ([new Fixture\FalseDefaultResponse('default'), new Fixture\FalseDefaultResponse('explicit', false, 0)] as $default) {
    if ($default->retryEnabled !== false || $default->count !== 0) {
        throw new Exception('Some(false)/Some(0) default changed');
    }
}
echo "7 constructor cases passed\n";
"#
}
