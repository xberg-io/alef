use crate::backends::dart::DartBackend;
use crate::cli::pipeline::{StagingProfile, run_post_build};
use crate::snippets::validators::{CapturedStreams, run_command_streams};
use std::path::Path;

mod dart_sdk {
    use crate as alef;
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/dart_sdk.rs"));
}
#[path = "u64_callers_native/emission.rs"]
mod emission;
#[path = "../../../../tests/support/mise_task.rs"]
mod mise_task;
#[path = "u64_callers_native/setup.rs"]
mod setup;

fn task(root: &Path, name: &str, arguments: &[&str], timeout: u64) -> CapturedStreams {
    let mut command = mise_task::mise_run(root, name);
    command
        .args(arguments)
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_TARGET_DIR", root.join("packages/dart/rust/target"));
    run_command_streams(&mut command, timeout).unwrap()
}

fn successful(root: &Path, name: &str, arguments: &[&str], timeout: u64) -> CapturedStreams {
    let output = task(root, name, arguments, timeout);
    assert!(output.success, "{name}: {}\n{}", output.stdout, output.stderr);
    output
}

#[test]
#[ignore = "requires managed Dart 3.13.7, FRB 2.13, cached Pub/Cargo dependencies and native Rust tools"]
fn actual_emitted_u64_callers_roundtrip_through_frb() {
    assert!(
        std::env::var("ALEF_SKIP_COMMANDS")
            .unwrap_or_default()
            .trim()
            .is_empty()
    );
    assert!(std::env::var_os("ALEF_FRB_REGENERATE_ON_BUILD").is_none());
    let directory = tempfile::Builder::new()
        .prefix("alef-dart-emitted-u64-")
        .tempdir()
        .unwrap();
    let root = directory.path().to_path_buf();
    let _cleanup = if std::env::var_os("ALEF_DART_CALLER_PROBE_KEEP").is_some() {
        let _ = directory.keep();
        None
    } else {
        Some(directory)
    };
    eprintln!("caller_fixture_root={}", root.display());
    let (api, config) = setup::fixture(&root);
    let core_before = std::fs::read(root.join("core/src/lib.rs")).unwrap();
    let parent = run_command_streams(std::process::Command::new("dart").arg("--version"), 30).unwrap();
    assert!(parent.success, "{}", parent.stderr);
    dart_sdk::assert_pinned_sdk(&parent, "caller_parent");
    dart_sdk::assert_pinned_sdk(&successful(&root, "sdk", &[], 30), "caller_fixture");
    successful(&root, "setup", &[], 600);
    let build = DartBackend.build_config_for(&config).unwrap();
    assert_eq!(build.post_build.len(), 15, "full canonical FRB pipeline");
    let outcome = run_post_build(
        crate::core::config::Language::Dart,
        &build,
        &config,
        &root,
        StagingProfile::NoBuildRequested,
    )
    .unwrap();
    assert!(
        outcome.skipped_missing_tools.is_empty(),
        "{:?}",
        outcome.skipped_missing_tools
    );
    let (program, poisoned) = emission::programs(&api, &config);
    setup::write(&root, "packages/dart/check.dart", &program);
    setup::write(&root, "packages/dart/poison.dart", &poisoned);
    successful(&root, "build", &[], 600);
    assert_eq!(
        std::fs::read(root.join("core/src/lib.rs")).unwrap(),
        core_before,
        "native generation/build preserves exact extraction input"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("packages/dart/check.dart")).unwrap(),
        program,
        "execute the actual emitted call artifact"
    );
    let native = root.join("packages/dart/rust/target/debug").join(format!(
        "{}wide_codec_dart{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(native.is_file() && native.metadata().unwrap().len() > 0);
    let native = native.to_str().unwrap();
    let negative = task(&root, "poison", &["--", native], 90);
    assert!(!negative.success, "the poisoned emitted integer must not compile");
    assert!(
        negative.stderr.contains("poison.dart")
            && negative
                .stderr
                .contains("The argument type 'int' can't be assigned to the parameter type 'BigInt'"),
        "wrong negative-control failure: {}\n{}",
        negative.stdout,
        negative.stderr
    );
    let positive = successful(&root, "test", &["--", native], 90);
    verify_receipts(&positive);
}

fn verify_receipts(positive: &CapturedStreams) {
    let receipts: Vec<serde_json::Value> = positive
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("caller_receipt="))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(receipts.len(), 15, "{}\n{}", positive.stdout, positive.stderr);
    let ids: std::collections::HashSet<_> = receipts.iter().map(|r| r["case"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 15);
    for kind in ["scalar", "nested", "optional_null", "signed", "u32", "usize", "json"] {
        let count = receipts.iter().filter(|r| r["kind"] == kind).count();
        assert_eq!(count, if matches!(kind, "scalar" | "nested") { 5 } else { 1 }, "{kind}");
    }
    assert!(positive.stdout.contains("caller_cases=15"), "{}", positive.stdout);
    eprintln!("{}", positive.stdout);
}
