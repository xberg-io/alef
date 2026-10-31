use alef::backends::dart::DartBackend;
use alef::cli::pipeline::{StagingProfile, run_post_build};
use alef::core::backend::{Backend, BuildConfig, PostBuildStep, PostProcessor};
use alef::core::config::{DartConfig, Language, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{PrimitiveType, TypeRef};
use alef::snippets::validators::{CapturedStreams, run_command_streams};
use std::path::Path;

#[path = "support/mise_task.rs"]
mod mise_task;

#[path = "support/dart_sdk.rs"]
mod dart_sdk;

const RAW_WRITER: &str = "serializer.buffer.putBigUint64(self);";
const GUARDED_WRITER: &str = "serializer.buffer.putBigUint64(_alefCheckU64BeforeEncode(self));";
const HELPER: &str = "BigInt _alefCheckU64BeforeEncode(BigInt value)";
const MODES: [&str; 8] = [
    "raw-max",
    "raw-negative",
    "raw-overflow",
    "signed",
    "callback-max",
    "callback-body-error",
    "callback-negative",
    "callback-overflow",
];

fn write(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn fixture(root: &Path) -> ResolvedCrateConfig {
    for (name, content) in [
        ("core/src/lib.rs", include_str!("fixtures/dart_u64_canonical/core.rs")),
        (
            "core/Cargo.toml",
            include_str!("fixtures/dart_u64_canonical/core_Cargo.toml"),
        ),
        (
            "packages/dart/rust/Cargo.toml",
            include_str!("fixtures/dart_u64_canonical/bridge_Cargo.toml"),
        ),
        (
            "packages/dart/pubspec.yaml",
            include_str!("fixtures/dart_u64_canonical/pubspec.yaml"),
        ),
        (
            "packages/dart/check.dart",
            include_str!("fixtures/dart_u64_canonical/check.dart"),
        ),
        ("mise.toml", include_str!("fixtures/dart_u64_canonical/mise.toml")),
        ("mise.lock", include_str!("fixtures/dart_u64_canonical/mise.lock")),
        (
            ".cargo/config.toml",
            include_str!("fixtures/dart_u64_canonical/cargo_config.toml"),
        ),
        ("rust-toolchain.toml", include_str!("../rust-toolchain.toml")),
    ] {
        write(root, name, content);
    }
    let source = root.join("core/src/lib.rs");
    let api = alef::extract::extractor::extract(&[&source], "wide-codec", "1.0.0", None).unwrap();
    let scalar = api.functions.iter().find(|f| f.name == "raw_scalar").unwrap();
    assert_eq!(scalar.params[0].ty, TypeRef::Primitive(PrimitiveType::U64));
    let callback = api
        .types
        .iter()
        .find(|t| t.name == "WideCallback" && t.is_trait)
        .unwrap();
    let method = callback.methods.iter().find(|m| m.name == "value").unwrap();
    assert_eq!(method.params[0].ty, TypeRef::Primitive(PrimitiveType::U64));
    assert_eq!(method.return_type, TypeRef::Primitive(PrimitiveType::U64));
    assert!(
        method.error_type.is_none() && !method.is_async,
        "preserve the existing infallible synchronous contract"
    );
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"wide-codec\"\nsources=[\"core/src/lib.rs\"]\n",
    )
    .unwrap();
    let mut config = config.resolve().unwrap().remove(0);
    config.dart = Some(DartConfig {
        pubspec_name: Some("wide_codec_probe".into()),
        frb_version: Some("2.13.0".into()),
        exclude_types: vec!["WideRegistry".into()],
        exclude_functions: vec!["get_registry".into()],
        ..Default::default()
    });
    config.trait_bridges.push(TraitBridgeConfig {
        trait_name: "WideCallback".into(),
        registry_getter: Some("wide_codec::plugins::registry::get_registry".into()),
        register_fn: Some("register_wide_callback".into()),
        ..Default::default()
    });
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let mut written = 0;
    for file in files {
        if file.path.starts_with("packages/dart/lib")
            || [
                "packages/dart/rust/src/lib.rs",
                "packages/dart/rust/build.rs",
                "packages/dart/rust/flutter_rust_bridge.yaml",
            ]
            .iter()
            .any(|path| file.path == Path::new(path))
        {
            write(root, file.path.to_str().unwrap(), &file.content);
            written += 1;
        }
    }
    assert!(
        written >= 5,
        "real generated facade, callback implementation, config and Dart files"
    );
    let facade = std::fs::read_to_string(root.join("packages/dart/rust/src/lib.rs")).unwrap();
    assert!(facade.contains("pub fn create_wide_callback_dart_impl("));
    assert!(facade.contains("pub fn register_wide_callback("));
    assert!(facade.contains("Fn(u64) -> DartFnFuture<u64>"));
    assert!(
        facade.contains("host callback panicked; returning default"),
        "freeze the existing error fallback policy"
    );
    config
}

fn task(root: &Path, name: &str, arguments: &[&str], seconds: u64) -> CapturedStreams {
    let mut command = mise_task::mise_run(root, name);
    command.args(arguments).env("RUSTC_WRAPPER", "");
    if let Some(cache) = std::env::var_os("CARGO_HOME") {
        command.env("CARGO_HOME", cache);
    }
    run_command_streams(&mut command, seconds).unwrap()
}

fn successful(root: &Path, name: &str, arguments: &[&str], seconds: u64) -> CapturedStreams {
    let output = task(root, name, arguments, seconds);
    assert!(output.success, "{name}: {}\n{}", output.stdout, output.stderr);
    output
}

fn post_build(root: &Path, config: &ResolvedCrateConfig, build: &BuildConfig) {
    let outcome = run_post_build(Language::Dart, build, config, root, StagingProfile::NoBuildRequested).unwrap();
    assert!(
        outcome.skipped_missing_tools.is_empty(),
        "canonical tool steps skipped: {:?}",
        outcome.skipped_missing_tools
    );
}

fn check_case(mode: &str, output: &CapturedStreams) -> Result<serde_json::Value, String> {
    if !output.success {
        return Err(format!("{mode}: {}\n{}", output.stdout, output.stderr));
    }
    let receipts: Vec<_> = output
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("canonical_receipt="))
        .collect();
    if receipts.len() != 1 {
        return Err(format!("{mode}: expected exactly one receipt: {}", output.stdout));
    }
    let receipt: serde_json::Value = serde_json::from_str(receipts[0]).map_err(|e| e.to_string())?;
    let qualified = matches!(mode, "raw-max" | "signed" | "callback-max");
    if receipt["mode"] != mode || receipt["qualified_roundtrip"] != qualified || receipt["uncaught_async"] != 0 {
        return Err(format!("{mode}: wrong qualification/count receipt: {receipt}"));
    }
    if mode.starts_with("callback-") && receipt["callbacks"] != 1 {
        return Err(format!("{mode}: expected exactly one actual Dart callback: {receipt}"));
    }
    if matches!(
        mode,
        "raw-negative" | "raw-overflow" | "callback-negative" | "callback-overflow"
    ) && receipt["rejected"] != true
    {
        return Err(format!("{mode}: missing actual range-rejection receipt: {receipt}"));
    }
    let fallback = matches!(mode, "callback-body-error" | "callback-negative" | "callback-overflow");
    if fallback {
        let cause = if mode == "callback-body-error" {
            "callback-body-control"
        } else {
            "u64 unsigned value"
        };
        if receipt["error_fallback"] != "warned-default-zero"
            || receipt["value"] != "0"
            || !output.stderr.contains("host callback panicked; returning default")
            || !output.stderr.contains(cause)
        {
            return Err(format!(
                "{mode}: missing explicit warned error fallback/cause: {receipt}\n{}",
                output.stderr
            ));
        }
    } else if output.stderr.contains("host callback panicked") {
        return Err(format!("{mode}: unexpected callback error fallback: {}", output.stderr));
    }
    Ok(receipt)
}

#[test]
#[ignore = "requires pinned FRB 2.13, Dart 3.13.7, cached Pub/Cargo dependencies and native Rust tools"]
fn canonical_frb_u64_raw_and_callback_writers_enforce_bounds() {
    assert!(
        std::env::var("ALEF_SKIP_COMMANDS")
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "canonical commands may not be skipped"
    );
    assert!(
        std::env::var_os("ALEF_FRB_REGENERATE_ON_BUILD").is_none(),
        "build scripts must not bypass canonical generation"
    );
    let directory = tempfile::Builder::new()
        .prefix("alef-dart-canonical-u64-")
        .tempdir()
        .unwrap();
    let root = directory.path().to_path_buf();
    let _cleanup = if std::env::var_os("ALEF_DART_CANONICAL_U64_KEEP").is_some() {
        let _ = directory.keep();
        None
    } else {
        Some(directory)
    };
    eprintln!("canonical_fixture_root={}", root.display());
    let config = fixture(&root);
    let sdk = successful(&root, "sdk", &[], 30);
    dart_sdk::assert_pinned_sdk(&sdk, "canonical");
    successful(&root, "setup", &[], 600);
    let canonical = DartBackend.build_config_for(&config).unwrap();
    assert_eq!(canonical.post_build.len(), 15, "full canonical FRB pipeline");
    assert_eq!(
        canonical
            .post_build
            .iter()
            .filter(|step| matches!(
                step,
                PostBuildStep::PostProcessFile {
                    processor: PostProcessor::FrbDartSealedVariants,
                    ..
                }
            ))
            .count(),
        2
    );
    let prefix = canonical
        .post_build
        .iter()
        .position(|step| matches!(step, PostBuildStep::PostProcessFile { .. }))
        .unwrap();
    assert_eq!(
        prefix, 3,
        "negative control retains version, real FRB generation and coverage gates"
    );
    let mut unprocessed = canonical.clone();
    unprocessed.post_build.truncate(prefix);
    post_build(&root, &config, &unprocessed);
    let bridge = root.join("packages/dart/lib/src/wide_codec_bridge_generated/frb_generated.dart");
    let before = std::fs::read_to_string(&bridge).unwrap();
    let writers = before.matches(RAW_WRITER).count();
    assert!(
        writers > 0 && !before.contains(HELPER),
        "actual unprocessed writer boundary must exist"
    );
    successful(&root, "build", &[], 600);
    let in_effect = std::fs::read_to_string(&bridge).unwrap();
    assert_eq!(
        in_effect.matches(RAW_WRITER).count(),
        writers,
        "negative control must run the actual unguarded artifact"
    );
    assert!(!in_effect.contains(HELPER));
    let native = root.join("packages/dart/rust/target/debug").join(format!(
        "{}wide_codec_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(
        native.is_file() && native.metadata().unwrap().len() > 0,
        "load the real newly built native library"
    );
    let native = native.to_str().unwrap();
    for mode in ["raw-negative", "raw-overflow"] {
        let negative = task(&root, "test", &["--", native, mode], 45);
        assert!(
            !negative.success
                && negative.stderr.contains("expected RangeError before native entry")
                && negative.stderr.contains("calls=1"),
            "wrong-reason negative {mode}: {}\n{}",
            negative.stdout,
            negative.stderr
        );
    }
    post_build(&root, &config, &canonical);
    let after = std::fs::read_to_string(&bridge).unwrap();
    assert_ne!(before, after);
    assert_eq!(after.matches(RAW_WRITER).count(), 0);
    assert_eq!(after.matches(GUARDED_WRITER).count(), writers);
    assert_eq!(after.matches(HELPER).count(), 1);
    successful(&root, "build", &[], 600);
    let in_effect = std::fs::read_to_string(&bridge).unwrap();
    assert_eq!(in_effect.matches(RAW_WRITER).count(), 0);
    assert_eq!(in_effect.matches(GUARDED_WRITER).count(), writers);
    assert_eq!(in_effect.matches(HELPER).count(), 1);
    let mut completed = 0;
    let mut receipts = Vec::new();
    let mut failures = Vec::new();
    for mode in MODES {
        let output = task(&root, "test", &["--", native, mode], 45);
        completed += 1;
        match check_case(mode, &output) {
            Ok(receipt) => receipts.push(receipt),
            Err(error) => failures.push(error),
        }
    }
    eprintln!(
        "canonical_cases_expected=8 completed_processes={} passed={} failed={} receipts={}",
        completed,
        receipts.len(),
        failures.len(),
        serde_json::to_string(&receipts).unwrap()
    );
    assert_eq!(completed, MODES.len());
    assert!(
        failures.is_empty(),
        "canonical unsigned qualification failures:\n{}",
        failures.join("\n\n")
    );
    assert_eq!(receipts.len(), 8);
    assert_eq!(receipts.iter().filter(|r| r["qualified_roundtrip"] == true).count(), 3);
    assert_eq!(
        receipts
            .iter()
            .filter(|r| r["error_fallback"] == "warned-default-zero")
            .count(),
        3
    );
}
