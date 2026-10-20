//! `BuildConfig` emission for the Dart backend's flutter_rust_bridge pipeline.

use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;

use super::make_config;

#[test]
fn build_config_for_frb_emits_post_process_file_step() {
    use alef::core::backend::{PostBuildStep, PostProcessor};
    use std::path::PathBuf;

    let config = make_config();
    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style must yield a BuildConfig");

    let post_process_steps: Vec<&PostBuildStep> = bc
        .post_build
        .iter()
        .filter(|s| matches!(s, PostBuildStep::PostProcessFile { .. }))
        .collect();

    assert_eq!(
        post_process_steps.len(),
        10,
        "FRB config must have ten PostProcessFile steps: (1) exclude_functions on lib.dart, \
         (2) sealed_variants on lib.dart, (3) resolved constructor defaults on lib.dart, \
         (4) exclude_functions on frb_generated.dart, \
         (5) sealed_variants on frb_generated.dart for the published-package native-lib loader, \
         (6) fix handler executor calls on frb_generated.dart, \
         (7) repair single-variant copyWith, (8-10) strip trailing whitespace from generated Dart files"
    );

    let lib_dart_path = PathBuf::from("packages")
        .join("dart")
        .join("lib")
        .join("src")
        .join("demo_crate_bridge_generated")
        .join("lib.dart");
    let frb_generated_path = PathBuf::from("packages")
        .join("dart")
        .join("lib")
        .join("src")
        .join("demo_crate_bridge_generated")
        .join("frb_generated.dart");
    let lib_freezed_path = PathBuf::from("packages")
        .join("dart")
        .join("lib")
        .join("src")
        .join("demo_crate_bridge_generated")
        .join("lib.freezed.dart");

    if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[0] {
        assert!(
            matches!(processor, PostProcessor::FrbDartExcludeFunctions(..)),
            "First PostProcessFile must use FrbDartExcludeFunctions processor"
        );
        assert_eq!(path, &lib_dart_path, "First PostProcessFile must target lib.dart");
    }

    if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[1] {
        assert_eq!(
            *processor,
            PostProcessor::FrbDartSealedVariants,
            "Second PostProcessFile must use FrbDartSealedVariants processor"
        );
        assert_eq!(path, &lib_dart_path, "Second PostProcessFile must target lib.dart");
    }

    assert!(matches!(
        post_process_steps[2],
        PostBuildStep::PostProcessFile {
            path,
            processor: PostProcessor::FrbDartOptionalFieldsWithDefaults,
        } if path == &lib_dart_path
    ));

    if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[3] {
        assert!(
            matches!(processor, PostProcessor::FrbDartExcludeFunctions(..)),
            "Fourth PostProcessFile must use FrbDartExcludeFunctions processor"
        );
        assert_eq!(
            path, &frb_generated_path,
            "Fourth PostProcessFile must target frb_generated.dart"
        );
    }

    if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[4] {
        assert_eq!(
            *processor,
            PostProcessor::FrbDartSealedVariants,
            "Fifth PostProcessFile must use FrbDartSealedVariants processor (for native-lib loader)"
        );
        assert_eq!(
            path, &frb_generated_path,
            "Fifth PostProcessFile must target frb_generated.dart for the loader injection"
        );
    }

    if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[5] {
        assert_eq!(
            *processor,
            PostProcessor::FrbDartFixHandlerExecutorCalls,
            "Sixth PostProcessFile must use FrbDartFixHandlerExecutorCalls processor"
        );
        assert_eq!(
            path, &frb_generated_path,
            "Sixth PostProcessFile must target frb_generated.dart for handler executor fixes"
        );
    }

    assert!(matches!(
        post_process_steps[6],
        PostBuildStep::PostProcessFile { path, processor: PostProcessor::FrbDartSingleVariantCopyWith }
            if path == &lib_freezed_path
    ));

    for (idx, expected_path) in [&lib_dart_path, &frb_generated_path, &lib_freezed_path]
        .into_iter()
        .enumerate()
    {
        if let PostBuildStep::PostProcessFile { path, processor } = post_process_steps[idx + 7] {
            assert_eq!(
                *processor,
                PostProcessor::DartStripTrailingWhitespace,
                "PostProcessFile {} must strip trailing whitespace",
                idx + 8
            );
            assert_eq!(
                path,
                expected_path,
                "PostProcessFile {} must target the expected generated Dart file",
                idx + 8
            );
        }
    }
}

#[test]
fn build_config_for_frb_run_command_precedes_post_process_file() {
    use alef::core::backend::PostBuildStep;

    let config = make_config();
    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style must yield a BuildConfig");

    let steps: Vec<&str> = bc
        .post_build
        .iter()
        .map(|s| match s {
            PostBuildStep::RunCommand { .. } => "RunCommand",
            PostBuildStep::PostProcessFile { .. } => "PostProcessFile",
            PostBuildStep::PatchFile { .. } => "PatchFile",
            PostBuildStep::CarryFrbCfgGates { .. } => "CarryFrbCfgGates",
            PostBuildStep::StageDartNatives { .. } => "StageDartNatives",
            PostBuildStep::StageFfiLibrary => "StageFfiLibrary",
            PostBuildStep::MaterializeSwiftBridge { .. } => "MaterializeSwiftBridge",
            PostBuildStep::RewriteWasmPackageName { .. } => "RewriteWasmPackageName",
            PostBuildStep::VerifyFrbBridgeCoverage { .. } => "VerifyFrbBridgeCoverage",
            PostBuildStep::VerifyFrbCodegenVersion { .. } => "VerifyFrbCodegenVersion",
        })
        .collect();

    assert_eq!(
        steps,
        vec![
            "VerifyFrbCodegenVersion",
            "RunCommand",
            "VerifyFrbBridgeCoverage",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "PostProcessFile",
            "CarryFrbCfgGates",
            "StageDartNatives"
        ],
        "VerifyFrbCodegenVersion must come before RunCommand (alef #204: never invoke a \
         flutter_rust_bridge_codegen whose version disagrees with the declared pin), which must \
         come before VerifyFrbBridgeCoverage, which must come before every PostProcessFile step \
         in post_build steps (alef #135: a PostProcessFile step must never patch a bridge that \
         VerifyFrbBridgeCoverage has not yet cleared as fresh)"
    );
}

#[test]
fn build_config_for_frb_verify_codegen_version_uses_the_declared_frb_version_pin() {
    use alef::core::backend::PostBuildStep;

    let config = make_config();
    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style must yield a BuildConfig");

    let expected_version = bc
        .post_build
        .iter()
        .find_map(|step| match step {
            PostBuildStep::VerifyFrbCodegenVersion { expected_version } => Some(expected_version.clone()),
            _ => None,
        })
        .expect("FRB config must include a VerifyFrbCodegenVersion step");

    assert_eq!(
        expected_version,
        alef::backends::dart::naming::dart_frb_version(&config),
        "VerifyFrbCodegenVersion must check against the same pin the generated \
         Cargo.toml/pubspec.yaml dependency declares -- a separate, disconnected pin would defeat \
         the point of the check"
    );
}

#[test]
fn build_config_for_frb_skip_frb_omits_verify_codegen_version() {
    use alef::core::backend::PostBuildStep;

    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.dart]
skip_frb = true
"#;
    let cfg: alef::core::config::new_config::NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style with skip_frb must still yield a BuildConfig");

    assert!(
        !bc.post_build
            .iter()
            .any(|s| matches!(s, PostBuildStep::VerifyFrbCodegenVersion { .. })),
        "skip_frb = true must suppress VerifyFrbCodegenVersion along with the RunCommand it guards"
    );
}

#[test]
fn build_config_for_frb_run_command_uses_config_file() {
    use alef::core::backend::PostBuildStep;

    let config = make_config();
    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style must yield a BuildConfig");

    let run_command = bc
        .post_build
        .iter()
        .find_map(|step| match step {
            PostBuildStep::RunCommand { cmd, args } => Some((*cmd, args)),
            _ => None,
        })
        .expect("FRB config must run flutter_rust_bridge_codegen");

    assert_eq!(run_command.0, "flutter_rust_bridge_codegen");
    assert_eq!(
        run_command.1,
        &vec![
            "generate",
            "--config-file",
            "packages/dart/rust/flutter_rust_bridge.yaml",
            "--no-deps-check"
        ],
        "flutter_rust_bridge_codegen must read the generated config file"
    );
}

#[test]
fn build_config_with_config_includes_post_build_steps() {
    use alef::core::backend::PostBuildStep;

    let config = make_config();
    let backend = DartBackend;

    let bc_with_config = backend
        .build_config_with_config(&config)
        .expect("build_config_with_config must return a BuildConfig");
    let bc_for = backend
        .build_config_for(&config)
        .expect("build_config_for must return a BuildConfig");

    assert_eq!(
        bc_with_config.post_build.len(),
        bc_for.post_build.len(),
        "build_config_with_config must have the same number of post-build steps as build_config_for"
    );

    let has_optional_fields_processor = bc_with_config.post_build.iter().any(|step| {
        if let PostBuildStep::PostProcessFile { processor, .. } = step {
            matches!(
                processor,
                alef::core::backend::PostProcessor::FrbDartOptionalFieldsWithDefaults
            )
        } else {
            false
        }
    });

    assert!(
        has_optional_fields_processor,
        "build_config_with_config must apply resolved constructor defaults"
    );
}

#[test]
fn build_config_for_frb_skip_frb_omits_run_command() {
    use alef::core::backend::PostBuildStep;
    use alef::core::config::languages::DartConfig;

    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.dart]
skip_frb = true
"#;
    let cfg: alef::core::config::new_config::NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style with skip_frb must still yield a BuildConfig");

    let has_run_command = bc
        .post_build
        .iter()
        .any(|s| matches!(s, PostBuildStep::RunCommand { .. }));
    assert!(
        !has_run_command,
        "skip_frb = true must suppress the flutter_rust_bridge_codegen RunCommand; \
         got steps: {:?}",
        bc.post_build
            .iter()
            .map(|s| match s {
                PostBuildStep::RunCommand { cmd, .. } => format!("RunCommand({cmd})"),
                PostBuildStep::PostProcessFile { .. } => "PostProcessFile".to_string(),
                PostBuildStep::PatchFile { .. } => "PatchFile".to_string(),
                PostBuildStep::CarryFrbCfgGates { .. } => "CarryFrbCfgGates".to_string(),
                PostBuildStep::StageDartNatives { lib_stem } => {
                    format!("StageDartNatives({lib_stem})")
                }
                PostBuildStep::StageFfiLibrary => "StageFfiLibrary".to_string(),
                PostBuildStep::MaterializeSwiftBridge { binding_crate_name, .. } => {
                    format!("MaterializeSwiftBridge({binding_crate_name})")
                }
                PostBuildStep::RewriteWasmPackageName { .. } => "RewriteWasmPackageName".to_string(),
                PostBuildStep::VerifyFrbBridgeCoverage { .. } => "VerifyFrbBridgeCoverage".to_string(),
                PostBuildStep::VerifyFrbCodegenVersion { .. } => "VerifyFrbCodegenVersion".to_string(),
            })
            .collect::<Vec<_>>()
    );

    let post_process_count = bc
        .post_build
        .iter()
        .filter(|s| matches!(s, PostBuildStep::PostProcessFile { .. }))
        .count();
    assert!(
        post_process_count > 0,
        "skip_frb = true must retain PostProcessFile steps for already-generated FRB output"
    );

    let dart_cfg: DartConfig = toml::from_str("skip_frb = true").expect("must parse");
    assert!(dart_cfg.skip_frb, "DartConfig.skip_frb must deserialise from TOML");

    let dart_default = DartConfig::default();
    assert!(!dart_default.skip_frb, "DartConfig.skip_frb must default to false");
}

#[test]
fn build_config_for_frb_emits_carry_frb_cfg_gates_step_with_rust_source_paths() {
    use alef::core::backend::PostBuildStep;
    use std::path::PathBuf;

    let config = make_config();
    let bc = DartBackend
        .build_config_for(&config)
        .expect("FRB style must yield a BuildConfig");

    let carry_steps: Vec<&PostBuildStep> = bc
        .post_build
        .iter()
        .filter(|s| matches!(s, PostBuildStep::CarryFrbCfgGates { .. }))
        .collect();

    assert_eq!(
        carry_steps.len(),
        1,
        "FRB config must schedule exactly one CarryFrbCfgGates step"
    );

    let expected_source = PathBuf::from("packages")
        .join("dart")
        .join("rust")
        .join("src")
        .join("lib.rs");
    let expected_target = PathBuf::from("packages")
        .join("dart")
        .join("rust")
        .join("src")
        .join("frb_generated.rs");

    if let PostBuildStep::CarryFrbCfgGates {
        source_path,
        target_path,
    } = carry_steps[0]
    {
        assert_eq!(
            source_path, &expected_source,
            "CarryFrbCfgGates must scan the FRB source crate's lib.rs"
        );
        assert_eq!(
            target_path, &expected_target,
            "CarryFrbCfgGates must rewrite the generated frb_generated.rs"
        );
    }
}
