use alef::codegen::component::{ResolvedComponent, resolve_components};
use alef::codegen::component_producer::generate_component_producers;
use alef::component::artifact::{PackageInput, ProvidedContractInput, create_manifest, feature_hash, write_package};
use alef::core::config::ResolvedCrateConfig;
use alef::core::ir::ApiSurface;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const IMPLEMENTATION: &str = r#"package main
import "errors"
type reverseCodec struct{}
func NewCodec() Codec { return &reverseCodec{} }
func (*reverseCodec) Transform(input []byte) ([]byte, error) {
    switch string(input) {
    case "error": return nil, errors.New("conformance error")
    case "panic": panic("conformance panic")
    }
    result := append([]byte(nil), input...)
    for left, right := 0, len(result)-1; left < right; left, right = left+1, right-1 {
        result[left], result[right] = result[right], result[left]
    }
    return result, nil
}
"#;

fn run(command: &mut Command) -> Output {
    command
        .output()
        .unwrap_or_else(|error| panic!("running {command:?}: {error}"))
}

fn successful(command: &mut Command) -> Output {
    let output = run(command);
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn build_go(root: &Path, mutation: Option<(&str, &str)>, api: &ApiSurface, config: &ResolvedCrateConfig) -> PathBuf {
    let files = generate_component_producers(api, config).expect("generate Go conformance producer");
    assert_eq!(
        files.len(),
        6,
        "expected five Go bridge/seed files and one contract header"
    );
    let mut directory = None;
    let mut mutation_count = 0;
    for file in files {
        let destination = root.join(&file.path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let mut content = file.content;
        if file.path.ends_with("implementation.go") {
            content = IMPLEMENTATION.into();
        }
        if file.path.ends_with("component.c") || file.path.ends_with("component.go") {
            if let Some((before, after)) = mutation {
                mutation_count += content.matches(before).count();
                content = content.replace(before, after);
            }
        }
        if file.path.ends_with("go.mod") {
            directory = Some(destination.parent().unwrap().to_path_buf());
        }
        std::fs::write(destination, content).unwrap();
    }
    assert_eq!(
        mutation_count,
        usize::from(mutation.is_some()),
        "negative control must mutate exactly one bridge fragment"
    );
    let directory = directory.expect("go.mod was emitted");
    let library = directory.join(format!("conformance.{}", std::env::consts::DLL_EXTENSION));
    successful(
        Command::new("mise")
            .args(["exec", "--", "go", "-C"])
            .arg(&directory)
            .args(["build", "-buildmode=c-shared", "-o"])
            .arg(&library)
            .arg(".")
            .env("CGO_ENABLED", "1")
            .current_dir(env!("CARGO_MANIFEST_DIR")),
    );
    library
}

fn package(root: &Path, library: &Path, component: &ResolvedComponent) -> PathBuf {
    let provided = &component.provides[0];
    let hash = provided
        .contract_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let provides = [ProvidedContractInput {
        contract: &provided.contract.name,
        interface_version: 1,
        contract_hash: &hash,
        implementation: &provided.implementation,
    }];
    let manifest = create_manifest(
        library,
        PackageInput {
            crate_name: "demo",
            component: &component.component_name,
            version: "1.2.3",
            target: "conformance-native",
            provides: &provides,
            features: &component.features,
            default_features: component.default_features,
            feature_hash: &feature_hash(&component.features, component.default_features),
        },
    )
    .unwrap();
    let output = root.join("package");
    let record = write_package(library, &output, manifest.clone(), None).unwrap();
    let entry = alef_component_runtime::ComponentLockEntry {
        identity: manifest.identity,
        provides: manifest.provides,
        mode: alef_component_runtime::ComponentDeliveryMode::Download,
        url: format!("file://{}", output.join(record.archive).display()),
        sha256: record.archive_sha256,
        size: record.archive_size,
        manifest_sha256: record.manifest_sha256,
        key_id: String::new(),
    };
    let lock = alef_component_runtime::ComponentLock {
        schema_version: alef_component_runtime::COMPONENT_MANIFEST_SCHEMA,
        public_keys: Default::default(),
        artifacts: vec![entry],
    };
    let path = root.join("components.lock.json");
    std::fs::write(&path, serde_json::to_vec(&lock).unwrap()).unwrap();
    path
}

fn driver(root: &Path, hash: [u8; 32]) -> PathBuf {
    let directory = root.join("driver");
    std::fs::create_dir_all(directory.join("src")).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = format!(
        r#"[package]
name = "component-conformance"
version = "0.0.0"
edition = "2024"
[dependencies]
alef-component-abi = {{ path = {:?} }}
alef-component-runtime = {{ path = {:?} }}
serde_json = "1"
[workspace]
"#,
        repo.join("crates/alef-component-abi"),
        repo.join("crates/alef-component-runtime")
    );
    std::fs::write(directory.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(directory.join("src/main.rs"), include_str!("main.rs")).unwrap();
    std::fs::write(directory.join("src/contract_hash.rs"), format!("{hash:?}")).unwrap();
    successful(
        Command::new("mise")
            .args(["exec", "--", "cargo", "build", "--manifest-path"])
            .arg(directory.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(directory.join("target")),
    );
    directory
        .join("target/debug")
        .join(format!("component-conformance{}", std::env::consts::EXE_SUFFIX))
}

pub fn qualify(api: &ApiSurface, config: &ResolvedCrateConfig) {
    let root = tempfile::Builder::new()
        .prefix("alef-go-component-conformance-")
        .tempdir()
        .unwrap()
        .keep();
    #[allow(clippy::print_stderr)]
    {
        eprintln!("Go component conformance artifacts: {}", root.display());
    }
    let resolved = resolve_components(api, config).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].provides.len(), 1);
    let executable = driver(&root, resolved[0].provides[0].contract_hash);
    let hash_fragment = format!(
        "{{ {} }}",
        resolved[0].provides[0]
            .contract_hash
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    let cases = [
        ("positive", None),
        ("wrong-hash", Some((hash_fragment.as_str(), "{ 0 }"))),
        (
            "wrong-output",
            Some((
                "{ptr, len, len, NULL, alef_go_free}",
                "{ptr, 0, len, NULL, alef_go_free}",
            )),
        ),
        ("missing-free", Some(("NULL, alef_go_free", "NULL, NULL"))),
        (
            "wrong-status",
            Some((
                "if err != nil { *outError = alefBuffer([]byte(err.Error())); return 3 }",
                "if err != nil { *outError = alefBuffer([]byte(err.Error())); return 0 }",
            )),
        ),
        (
            "missing-method",
            Some(("sizeof(CodecApiV1), AlefGoCodecTransform", "sizeof(CodecApiV1), NULL")),
        ),
        (
            "wrong-descriptor",
            Some(("sizeof(AlefComponentV1), 1, 0", "sizeof(AlefComponentV1), 99, 0")),
        ),
    ];
    let mut observed = 0;
    for (name, mutation) in cases {
        let case = root.join(name);
        let library = build_go(&case, mutation, api, config);
        let lock = package(&case, &library, &resolved[0]);
        let output = run(Command::new(&executable).arg(lock).arg(case.join("cache")).args([
            "conformance-native",
            "reverse",
            "codec",
        ]));
        assert_eq!(
            output.status.success(),
            mutation.is_none(),
            "case {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if mutation.is_none() {
            assert!(String::from_utf8_lossy(&output.stdout).contains("COMPONENT_CONFORMANCE_OK"));
        } else {
            let reason = match name {
                "wrong-hash" => "ContractHashMismatch",
                "wrong-output" => "conformance result bytes",
                "missing-free" => "nonempty owned buffer must provide a free callback",
                "wrong-status" => "status for",
                "missing-method" => "bytes contract method is required",
                "wrong-descriptor" => "IncompatibleAbi",
                _ => unreachable!(),
            };
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(reason),
                "case {name} failed for wrong reason: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        observed += 1;
    }
    assert_eq!(observed, 7, "positive and every negative artifact must be exercised");
}
