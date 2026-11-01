use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ARCHIVE: &str = "TestLib-rs.artifactbundle.zip";
const RECEIPT: &str = "TestLib-rs.artifactbundle.release.json";

struct Fixture {
    _temporary: tempfile::TempDir,
    work: PathBuf,
    bin: PathBuf,
}

fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
    std::fs::write(path, bytes).expect("write fixture");
}

fn checksum(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Fixture {
    fn new() -> Self {
        let temporary = if let Some(root) = std::env::var_os("ALEF_TEST_SWIFT_FIXTURES") {
            std::fs::create_dir_all(&root).expect("fixture root");
            tempfile::tempdir_in(root).expect("temporary fixture")
        } else {
            tempfile::tempdir().expect("temporary fixture")
        };
        let work = temporary.path().join("work");
        let bin = temporary.path().join("bin");
        std::fs::create_dir_all(work.join("crates/testlib-swift")).expect("Swift crate");
        std::fs::create_dir_all(&bin).expect("fixture tools");
        write(
            work.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = []\n[workspace.package]\nversion = \"2.0.0\"\n",
        );
        write(
            work.join("alef.toml"),
            "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"Cargo.toml\"\n[crates.swift]\nmodule_name = \"TestLib\"\n[crates.scaffold]\nrepository = \"https://github.com/example/testlib\"\n",
        );
        write(
            work.join("crates/testlib-swift/Cargo.toml"),
            "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n",
        );
        write(
            work.join("Package.swift"),
            "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"TestLib\", targets: [.binaryTarget(name: \"RustBridgeBinary\", url: \"https://github.com/example/testlib/releases/download/v2.0.0/TestLib-rs.artifactbundle.zip\", checksum: \"__ALEF_SWIFT_CHECKSUM__\")])\n",
        );
        write(work.join(".gitignore"), "/dist/\n/target/\n/.alef/\n");
        install_blocked_cargo(&bin);
        let fixture = Self {
            _temporary: temporary,
            work,
            bin,
        };
        let warmup = fixture.run(&["--regen", "--skip-swift-checksum"]);
        assert!(warmup.status.success(), "{}", String::from_utf8_lossy(&warmup.stderr));
        fixture.initialize_git();
        fixture
    }

    fn initialize_git(&self) {
        self.git(&["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Swift fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            self.git(&["config", key, value]);
        }
        let names = self.git(&["ls-files", "--others", "--exclude-standard"]);
        for path in names.lines() {
            self.git(&["add", "--", path]);
        }
        self.git(&["commit", "-m", "test: clean preparation source"]);
        assert!(self.git(&["status", "--porcelain"]).is_empty());
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.work)
            .output()
            .expect("Git fixture");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).expect("Git output").trim().to_string()
    }

    fn run(&self, args: &[&str]) -> Output {
        let binary = std::env::var_os("ALEF_TEST_SWIFT_CLI").unwrap_or_else(|| env!("CARGO_BIN_EXE_alef").into());
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").expect("PATH")));
        Command::new(binary)
            .arg("sync-versions")
            .arg("--no-color")
            .args(args)
            .current_dir(&self.work)
            .env("PATH", std::env::join_paths(paths).expect("fixture PATH"))
            .output()
            .expect("actual Alef CLI")
    }

    fn stage(&self) -> Value {
        let directory = self.work.join("dist/swift-artifactbundle");
        std::fs::create_dir_all(&directory).expect("staging");
        let file = std::fs::File::create(directory.join(ARCHIVE)).expect("ZIP");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("TestLib-rs.artifactbundle/info.json", options)
            .expect("metadata entry");
        zip.write_all(json!({"schemaVersion":"1.0","artifacts":{"RustBridgeBinary":{"type":"staticLibrary","version":"1.0.0","variants":[{"path":"macos/lib.a","supportedTriples":["arm64-apple-macosx"]}]}}}).to_string().as_bytes()).expect("metadata");
        zip.start_file("TestLib-rs.artifactbundle/macos/lib.a", options)
            .expect("library entry");
        zip.write_all(b"!<arch>\nfixture\n").expect("library");
        zip.finish().expect("finish ZIP");
        let receipt = json!({"schema":1,"repository":"example/testlib","source":self.git(&["rev-parse","HEAD"]),"version":"2.0.0","archive":ARCHIVE,"sha256":checksum(&std::fs::read(directory.join(ARCHIVE)).expect("ZIP bytes")),"triples":["arm64-apple-macosx"],"artifact_version":"1.0.0"});
        self.receipt(&receipt);
        receipt
    }

    fn receipt(&self, value: &Value) {
        write(
            self.work.join("dist/swift-artifactbundle").join(RECEIPT),
            value.to_string(),
        );
    }

    fn source(&self) -> BTreeMap<String, Vec<u8>> {
        self.git(&["ls-files"])
            .lines()
            .map(|path| {
                (
                    path.to_string(),
                    std::fs::read(self.work.join(path)).expect("tracked source"),
                )
            })
            .collect()
    }

    fn reject(&self, args: &[&str], message: &str) {
        let before = self.source();
        let status = self.git(&["status", "--porcelain", "--untracked-files=normal"]);
        let head = self.git(&["rev-parse", "HEAD"]);
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "invalid prebuilt reuse succeeded: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(self.source(), before, "rejection must precede source writes");
        assert_eq!(self.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(self.git(&["status", "--porcelain", "--untracked-files=normal"]), status);
        assert!(!self.work.join("target/alef-swift-checksum.txt").exists());
    }
}

fn install_blocked_cargo(bin: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let cargo = bin.join("cargo");
        write(&cargo, "#!/bin/sh\nexit 127\n");
        std::fs::set_permissions(cargo, std::fs::Permissions::from_mode(0o755)).expect("cargo guard");
    }
    #[cfg(windows)]
    write(bin.join("cargo.cmd"), "@exit /B 127\r\n");
}

#[test]
fn valid_receipt_pins_exact_zip_digest_with_independent_metadata_version() {
    let fixture = Fixture::new();
    let receipt = fixture.stage();
    let output = fixture.run(&["--regen"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let manifest = std::fs::read_to_string(fixture.work.join("Package.swift")).expect("manifest");
    let digest = receipt["sha256"].as_str().expect("digest");
    assert!(manifest.contains(&format!("checksum: \"{digest}\"")), "{manifest}");
    assert!(!manifest.contains("__ALEF_SWIFT_CHECKSUM__"));
    record_positive(&receipt, &manifest);
    assert_eq!(
        std::fs::read_to_string(fixture.work.join("target/alef-swift-checksum.txt")).expect("sidecar"),
        digest
    );
}

fn record_positive(receipt: &Value, manifest: &str) {
    let Some(root) = std::env::var_os("ALEF_TEST_SWIFT_FIXTURES") else {
        return;
    };
    let root = PathBuf::from(root);
    write(root.join("positive-Package.swift"), manifest);
    write(root.join("positive-receipt.json"), receipt.to_string());
    write(
        root.join("positive-result.json"),
        json!({
            "archive_sha256": receipt["sha256"],
            "manifest_sha256": checksum(manifest.as_bytes()),
            "receipt_source": receipt["source"],
            "product_version": receipt["version"],
            "artifact_metadata_version": receipt["artifact_version"],
            "platform_count": receipt["triples"].as_array().expect("triples").len(),
        })
        .to_string(),
    );
}

macro_rules! receipt_negative {
    ($name:ident, $field:literal, $value:expr, $message:literal) => {
        #[test]
        fn $name() {
            let fixture = Fixture::new();
            let mut receipt = fixture.stage();
            receipt[$field] = json!($value);
            fixture.receipt(&receipt);
            fixture.reject(&["--regen"], $message);
        }
    };
}
receipt_negative!(
    stale_version_is_rejected_before_writes,
    "version",
    "1.9.9",
    "version must match"
);
receipt_negative!(
    wrong_source_is_rejected_before_writes,
    "source",
    "0".repeat(40),
    "source differs"
);
receipt_negative!(
    wrong_repository_is_rejected_before_writes,
    "repository",
    "other/repo",
    "repository mismatch"
);
receipt_negative!(
    wrong_archive_is_rejected_before_writes,
    "archive",
    "wrong.zip",
    "archive mismatch"
);
receipt_negative!(
    wrong_digest_is_rejected_before_writes,
    "sha256",
    "f".repeat(64),
    "digest mismatch"
);
receipt_negative!(
    wrong_platform_receipt_is_rejected_before_writes,
    "triples",
    ["other-triple"],
    "platform declarations"
);
receipt_negative!(
    wrong_metadata_version_is_rejected_before_writes,
    "artifact_version",
    "2.0.0",
    "metadata version differs"
);

#[test]
fn wrong_named_zip_is_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    let directory = fixture.work.join("dist/swift-artifactbundle");
    std::fs::rename(directory.join(ARCHIVE), directory.join("wrong.zip")).expect("rename ZIP");
    fixture.reject(&["--regen"], "exactly one canonical ZIP");
}

#[test]
fn multiple_zips_are_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    write(fixture.work.join("dist/swift-artifactbundle/extra.zip"), b"extra");
    fixture.reject(&["--regen"], "exactly one canonical ZIP");
}

#[test]
fn missing_receipt_is_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    std::fs::remove_file(fixture.work.join("dist/swift-artifactbundle").join(RECEIPT)).expect("remove receipt");
    fixture.reject(&["--regen"], "exactly one canonical ZIP");
}

#[test]
fn receipt_without_zip_is_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    std::fs::remove_file(fixture.work.join("dist/swift-artifactbundle").join(ARCHIVE)).expect("remove ZIP");
    fixture.reject(&["--regen"], "exactly one canonical ZIP");
}

#[test]
fn dirty_preparation_source_is_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    let path = fixture.work.join("Cargo.toml");
    let mut source = std::fs::read_to_string(&path).expect("source");
    source.push_str("# changed preparation input\n");
    write(path, source);
    fixture.reject(&["--regen"], "clean preparation source");
}

#[test]
fn set_different_version_is_rejected_before_setter_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    fixture.reject(&["--regen", "--set", "2.1.0"], "version must match");
}

#[test]
fn bump_different_version_is_rejected_before_writes() {
    let fixture = Fixture::new();
    fixture.stage();
    fixture.reject(&["--regen", "--bump", "patch"], "version must match");
}

#[test]
fn source_only_without_artifacts_keeps_best_effort_placeholder_behavior() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.work.join("dist/swift-artifactbundle")).expect("empty staging");
    let output = fixture.run(&["--regen"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        std::fs::read_to_string(fixture.work.join("Package.swift"))
            .expect("manifest")
            .contains("__ALEF_SWIFT_CHECKSUM__")
    );
}

#[test]
fn explicit_skip_bypasses_leftover_prebuilt_files() {
    let fixture = Fixture::new();
    fixture.stage();
    write(fixture.work.join("dist/swift-artifactbundle/extra.zip"), b"leftover");
    let output = fixture.run(&["--regen", "--skip-swift-checksum"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        std::fs::read_to_string(fixture.work.join("Package.swift"))
            .expect("manifest")
            .contains("__ALEF_SWIFT_CHECKSUM__")
    );
}

#[test]
fn no_regeneration_does_not_consume_prebuilt_files() {
    let fixture = Fixture::new();
    fixture.stage();
    write(fixture.work.join("dist/swift-artifactbundle/extra.zip"), b"leftover");
    assert!(fixture.run(&[]).status.success());
    assert!(!fixture.work.join("target/alef-swift-checksum.txt").exists());
}

#[test]
fn directory_cannot_masquerade_as_the_zip() {
    let fixture = Fixture::new();
    fixture.stage();
    let path = fixture.work.join("dist/swift-artifactbundle").join(ARCHIVE);
    std::fs::remove_file(&path).expect("remove ZIP");
    std::fs::create_dir(path).expect("fake ZIP directory");
    fixture.reject(&["--regen"], "ZIP must be a regular file");
}

#[cfg(unix)]
#[test]
fn symlink_cannot_masquerade_as_the_zip() {
    let fixture = Fixture::new();
    fixture.stage();
    let path = fixture.work.join("dist/swift-artifactbundle").join(ARCHIVE);
    let payload = fixture.bin.join("payload.zip");
    std::fs::rename(&path, &payload).expect("move payload");
    std::os::unix::fs::symlink(payload, path).expect("fake ZIP symlink");
    fixture.reject(&["--regen"], "ZIP must be a regular file");
}

#[test]
fn corrupt_library_crc_is_rejected_before_source_writes() {
    let fixture = Fixture::new();
    let mut receipt = fixture.stage();
    let path = fixture.work.join("dist/swift-artifactbundle").join(ARCHIVE);
    let mut bytes = std::fs::read(&path).expect("stored ZIP");
    let payload = b"!<arch>\nfixture\n";
    let starts: Vec<_> = bytes
        .windows(payload.len())
        .enumerate()
        .filter_map(|(offset, value)| (value == payload).then_some(offset))
        .collect();
    assert_eq!(
        starts.len(),
        1,
        "the control must mutate the referenced library payload exactly once"
    );
    bytes[starts[0] + b"!<arch>\n".len()] ^= 1;
    let digest = checksum(&bytes);
    assert_ne!(receipt["sha256"].as_str(), Some(digest.as_str()));
    write(path, bytes);
    receipt["sha256"] = json!(digest);
    fixture.receipt(&receipt);
    let before = fixture.source();
    let status = fixture.git(&["status", "--porcelain", "--untracked-files=normal"]);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let output = fixture.run(&["--regen"]);
    let manifest = std::fs::read_to_string(fixture.work.join("Package.swift")).expect("manifest");
    let pinned = manifest.contains(&format!("checksum: \"{digest}\""));
    if let Some(root) = std::env::var_os("ALEF_TEST_SWIFT_FIXTURES") {
        write(
            PathBuf::from(root).join("crc-result.json"),
            json!({
                "exit": output.status.code(), "corrupted_zip_digest": digest, "pinned": pinned,
                "manifest_sha256": checksum(manifest.as_bytes()),
                "diagnostic": String::from_utf8_lossy(&output.stderr),
            })
            .to_string(),
        );
    }
    assert!(
        !output.status.success(),
        "CRC-corrupt library succeeded, checksum pinned={pinned}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Swift bundle member integrity"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!pinned, "a corrupt payload must not supply the manifest checksum");
    assert_eq!(fixture.source(), before);
    assert_eq!(
        fixture.git(&["status", "--porcelain", "--untracked-files=normal"]),
        status
    );
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert!(!fixture.work.join("target/alef-swift-checksum.txt").exists());
}
