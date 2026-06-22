use super::{ZigBuildFlags, render_build_zig, render_build_zig_zon};
use crate::e2e::config::DependencyMode;
use std::collections::BTreeMap;

/// Registry mode test_app build.zig must NOT reference `../../target/release`
/// (the local workspace layout). Instead, it must link the FFI from the
/// fetched package's bundled lib/include directories, ensuring compatibility
/// with published tarballs.
#[test]
fn registry_mode_build_zig_links_ffi_from_bundled_paths() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Registry,
        false,
        &std::collections::HashMap::new(),
        &[],
        &[],
    );

    assert!(
        !content.contains("../../target/release"),
        "registry mode build.zig must not reference workspace target dir, got:\n{content}"
    );
    assert!(
        content.contains("demo_client_dep.path(\"lib\")"),
        "registry mode build.zig must resolve FFI library path from fetched package's lib/ dir, got:\n{content}"
    );
    assert!(
        content.contains("demo_client_dep.path(\"include\")"),
        "registry mode build.zig must resolve FFI header path from fetched package's include/ dir, got:\n{content}"
    );
    assert!(
        content.contains("linkSystemLibrary(\"demo_client_ffi\""),
        "registry mode build.zig must link the FFI system library, got:\n{content}"
    );
}

/// Local mode test_app build.zig may reference `../../target/release` and
/// workspace-relative FFI paths (required for local development).
#[test]
fn local_mode_build_zig_uses_workspace_paths() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &std::collections::HashMap::new(),
        &[],
        &[],
    );

    assert!(
        content.contains("../../target/release"),
        "local mode build.zig must reference workspace target dir for local development, got:\n{content}"
    );
    assert!(
        content.contains("linkSystemLibrary(\"demo_client_ffi\""),
        "local mode build.zig must link the FFI system library, got:\n{content}"
    );
}

/// Non-empty env vars are injected via setEnvironmentVariable in alphabetical
/// order after addRunArtifact, and keys are sorted.
#[test]
fn env_vars_injected_alphabetically_after_run_artifact() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let mut env = std::collections::HashMap::new();
    env.insert("ZEBRA_VAR".to_string(), "z_value".to_string());
    env.insert("ALPHA_VAR".to_string(), "a_value".to_string());
    env.insert("BETA_VAR".to_string(), "b_value".to_string());

    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &env,
        &[],
        &[],
    );

    assert!(
        content.contains("setEnvironmentVariable(\"ALPHA_VAR\", \"a_value\")"),
        "env var ALPHA_VAR not found"
    );
    assert!(
        content.contains("setEnvironmentVariable(\"BETA_VAR\", \"b_value\")"),
        "env var BETA_VAR not found"
    );
    assert!(
        content.contains("setEnvironmentVariable(\"ZEBRA_VAR\", \"z_value\")"),
        "env var ZEBRA_VAR not found"
    );

    let alpha_pos = content.find("ALPHA_VAR").expect("ALPHA_VAR not found");
    let beta_pos = content.find("BETA_VAR").expect("BETA_VAR not found");
    let zebra_pos = content.find("ZEBRA_VAR").expect("ZEBRA_VAR not found");
    assert!(
        alpha_pos < beta_pos && beta_pos < zebra_pos,
        "env vars not in alphabetical order: ALPHA at {}, BETA at {}, ZEBRA at {}",
        alpha_pos,
        beta_pos,
        zebra_pos
    );
}

/// Empty env produces no setEnvironmentVariable calls.
#[test]
fn empty_env_produces_no_env_block() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let env = std::collections::HashMap::new();

    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &env,
        &[],
        &[],
    );

    let lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            line.contains("setEnvironmentVariable")
                && !line.contains("if (mock_server")
                && !line.contains("_entry.key_ptr")
        })
        .collect();
    assert!(
        lines.is_empty(),
        "empty env must not emit unconditional setEnvironmentVariable calls, got: {:?}",
        lines
    );
}

/// Test step dependency sequencing must not duplicate _run suffix.
/// Regression test for bug where prev_run already contains _run, but code appended _run again.
#[test]
fn test_step_dependencies_do_not_duplicate_run_suffix() {
    let test_filenames = vec![
        "first_test.zig".to_string(),
        "second_test.zig".to_string(),
        "third_test.zig".to_string(),
    ];
    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &std::collections::HashMap::new(),
        &[],
        &[],
    );

    assert!(
        !content.contains("_run_run"),
        "test step dependency must not contain '_run_run' (double suffix bug), but found in:\n{}",
        content
    );
}

/// Local mode build.zig.zon with a single harness_extras entry includes
/// the dependency in .dependencies with .url and .hash.
#[test]
fn local_mode_build_zig_zon_with_harness_extras_single_entry() {
    let capsule_deps = vec![(
        "tree_sitter".to_string(),
        "https://github.com/example/zig-tree-sitter/archive/refs/tags/v0.25.0.tar.gz".to_string(),
        "1220abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1".to_string(),
    )];

    let zon = render_build_zig_zon(
        "demo_client",
        "../../packages/zig",
        DependencyMode::Local,
        "0.1.0",
        &BTreeMap::new(),
        false,
        &capsule_deps,
    );

    assert!(
        zon.contains(".tree_sitter ="),
        "zon must contain tree_sitter dependency, got:\n{zon}"
    );
    assert!(
        zon.contains("https://github.com/example/zig-tree-sitter/archive/refs/tags/v0.25.0.tar.gz"),
        "zon must contain tree_sitter URL, got:\n{zon}"
    );
    assert!(
        zon.contains("1220abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1"),
        "zon must contain tree_sitter hash, got:\n{zon}"
    );
}

/// Local mode build.zig with harness_extras includes addImport wiring
/// for each dependency, allowing the binding module to @import the extras.
#[test]
fn local_mode_build_zig_with_harness_extras_add_import_wiring() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let capsule_deps = vec![(
        "tree_sitter".to_string(),
        "https://github.com/example/zig-tree-sitter/archive/refs/tags/v0.25.0.tar.gz".to_string(),
        "1220abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1".to_string(),
    )];

    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &std::collections::HashMap::new(),
        &capsule_deps,
        &[],
    );

    assert!(
        content.contains("const tree_sitter_dep = b.dependency(\"tree_sitter\""),
        "build.zig must fetch tree_sitter dependency, got:\n{content}"
    );
    assert!(
        content.contains("demo_client_module.addImport(\"tree_sitter\", tree_sitter_dep.module(\"tree_sitter\"))"),
        "build.zig must wire addImport for tree_sitter, got:\n{content}"
    );
}

/// Multiple harness_extras entries are all included in zon and build.zig,
/// without duplicates. The caller in mod.rs sorts before calling render_build_zig_zon.
#[test]
fn local_mode_build_zig_zon_with_multiple_harness_extras() {
    let capsule_deps = vec![
        (
            "alpha_lib".to_string(),
            "https://github.com/example/zig-alpha/archive/refs/tags/v2.0.0.tar.gz".to_string(),
            "1220aaa123aaa123aaa123aaa123aaa123aaa123aaa123aaa123aaa123aaa1".to_string(),
        ),
        (
            "zebra_lib".to_string(),
            "https://github.com/example/zig-zebra/archive/refs/tags/v1.0.0.tar.gz".to_string(),
            "1220zzz123zzz123zzz123zzz123zzz123zzz123zzz123zzz123zzz123zzz1".to_string(),
        ),
    ];

    let zon = render_build_zig_zon(
        "demo_client",
        "../../packages/zig",
        DependencyMode::Local,
        "0.1.0",
        &BTreeMap::new(),
        false,
        &capsule_deps,
    );

    assert!(zon.contains(".alpha_lib ="), "zon must include alpha_lib, got:\n{zon}");
    assert!(zon.contains(".zebra_lib ="), "zon must include zebra_lib, got:\n{zon}");
    let deps_start = zon.find(".dependencies =").expect("no .dependencies block");
    let deps_section = &zon[deps_start..];
    let alpha_pos = deps_section
        .find(".alpha_lib")
        .expect("alpha_lib not found in dependencies");
    let zebra_pos = deps_section
        .find(".zebra_lib")
        .expect("zebra_lib not found in dependencies");
    assert!(
        alpha_pos < zebra_pos,
        "dependencies must appear in order within .dependencies block: alpha at {}, zebra at {}",
        alpha_pos,
        zebra_pos
    );
}

/// Configured extra system libraries are linked alongside the FFI library
/// at the FFI link site, in both Local and Registry dependency modes.
#[test]
fn extra_system_libs_are_linked_in_both_modes() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let extra = vec!["heif".to_string()];

    for dep_mode in [DependencyMode::Local, DependencyMode::Registry] {
        let content = render_build_zig(
            &test_filenames,
            "demo_client",
            "demo_client",
            "demo_client_ffi",
            "../../crates/demo-client-ffi",
            ZigBuildFlags {
                has_file_fixtures: false,
                needs_mock_server: false,
            },
            "test_documents",
            dep_mode,
            false,
            &std::collections::HashMap::new(),
            &[],
            &extra,
        );

        assert!(
            content.contains("linkSystemLibrary(\"heif\", .{})"),
            "{dep_mode:?} mode build.zig must link the configured extra system library, got:\n{content}"
        );
        assert!(
            content.contains("linkSystemLibrary(\"demo_client_ffi\", .{})"),
            "{dep_mode:?} mode build.zig must still link the FFI library, got:\n{content}"
        );
    }
}

/// Multiple extra system libraries each emit a dedicated link call.
#[test]
fn multiple_extra_system_libs_each_linked() {
    let test_filenames = vec!["basic_test.zig".to_string()];
    let extra = vec!["heif".to_string(), "jpeg".to_string()];

    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &std::collections::HashMap::new(),
        &[],
        &extra,
    );

    assert!(
        content.contains("linkSystemLibrary(\"heif\", .{})"),
        "build.zig must link heif, got:\n{content}"
    );
    assert!(
        content.contains("linkSystemLibrary(\"jpeg\", .{})"),
        "build.zig must link jpeg, got:\n{content}"
    );
}

/// Default (empty) extra system libs emit no extra link calls, leaving the
/// demo_client_ffi output byte-identical to the existing expectation.
#[test]
fn empty_extra_system_libs_emit_no_extra_links() {
    let test_filenames = vec!["basic_test.zig".to_string()];

    let content = render_build_zig(
        &test_filenames,
        "demo_client",
        "demo_client",
        "demo_client_ffi",
        "../../crates/demo-client-ffi",
        ZigBuildFlags {
            has_file_fixtures: false,
            needs_mock_server: false,
        },
        "test_documents",
        DependencyMode::Local,
        false,
        &std::collections::HashMap::new(),
        &[],
        &[],
    );

    assert!(
        !content.contains("linkSystemLibrary(\"heif\""),
        "default (empty) extra system libs must not link heif, got:\n{content}"
    );
    let link_count = content.matches("linkSystemLibrary(").count();
    assert_eq!(
        link_count, 1,
        "default config must emit exactly one linkSystemLibrary call (the FFI lib), got {link_count}:\n{content}"
    );
}

/// Empty harness_extras does not emit spurious dependency entries.
#[test]
fn local_mode_build_zig_zon_without_harness_extras_is_unchanged() {
    let empty_capsule_deps: Vec<(String, String, String)> = vec![];

    let zon = render_build_zig_zon(
        "demo_client",
        "../../packages/zig",
        DependencyMode::Local,
        "0.1.0",
        &BTreeMap::new(),
        false,
        &empty_capsule_deps,
    );

    assert!(
        zon.contains(".demo_client ="),
        "zon must include main package, got:\n{zon}"
    );
    assert!(
        !zon.contains(".tree_sitter ="),
        "zon must not include harness_extras when empty, got:\n{zon}"
    );
}
