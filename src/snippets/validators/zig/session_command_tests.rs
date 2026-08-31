//! Regression coverage for the `zig build --build-file` command path in
//! `ZigValidator::validate_in_session`.
//!
//! The existing session tests in `zig.rs` (`sample_project`, `a_snippet_compiles_against_the_include_path...`)
//! only ever produce a bare manifest — one with `build.zig` but no `build.zig.zon` next to the
//! module source — so `zig_package_root` always returns `None` there and the reconstructed
//! command is always `zig build-exe`. None of them exercise the `zig build --build-file` branch,
//! which is exactly the path that regressed: `-I` is a `build-exe` flag and `zig build` rejects
//! it outright with `unrecognized argument: '-I'`. ~keep

use super::*;
use crate::snippets::types::{SnippetMetadata, SnippetStatus, SourceOrigin};

const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

fn package_fingerprint(name: &[u8]) -> u64 {
    let name_crc = crc32_ieee(name);
    let mut id: u32 = 0x811c_9dc5;
    for byte in name {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    ((name_crc as u64) << 32) | (id as u64)
}

/// A real zig *package* (`build.zig` **and** `build.zig.zon` at the same root) whose module
/// needs no include directory at all — the point is to prove that a nonempty
/// `session.include_paths` never reaches the `zig build` argv, not to exercise `@cInclude`.
///
/// This is what makes `zig_package_root` return `Some`, which is what selects the
/// `zig build --build-file` reconstruction over `zig build-exe`.
fn build_system_project() -> (tempfile::TempDir, ValidationSession) {
    let directory = tempfile::tempdir().expect("project directory");
    let root = directory.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("build.zig"),
        "const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n    \
         const target = b.standardTargetOptions(.{});\n    \
         const optimize = b.standardOptimizeOption(.{});\n    \
         _ = b.addModule(\"sample_binding\", .{\n        \
             .root_source_file = b.path(\"src/root.zig\"),\n        \
             .target = target,\n        \
             .optimize = optimize,\n    \
         });\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/root.zig"), "pub fn value() i32 {\n    return 42;\n}\n").unwrap();
    let fingerprint = package_fingerprint(b"sample_binding");
    std::fs::write(
        root.join("build.zig.zon"),
        format!(
            ".{{\n    .name = .sample_binding,\n    .version = \"0.0.0\",\n    \
             .fingerprint = 0x{fingerprint:016x},\n    \
             .minimum_zig_version = \"0.16.0\",\n    \
             .paths = .{{ \"build.zig\", \"build.zig.zon\", \"src\" }},\n}}\n"
        ),
    )
    .unwrap();

    let session = ValidationSession {
        language: Language::Zig,
        working_directory: root.to_path_buf(),
        manifest: Some(root.join("build.zig")),
        fingerprint: "build-system-project".into(),
        env: std::collections::BTreeMap::new(),
        // Nonempty and pointing at a directory that does not even exist: proving this test
        // requires no `-I` ever reaching the command, not merely a `-I` that happens to resolve.
        include_paths: vec![root.join("vendor/include")],
        rust_features: Vec::new(),
        rust_dependencies: std::collections::BTreeMap::new(),
    };
    (directory, session)
}

fn build_system_snippet() -> Snippet {
    let path = std::path::PathBuf::from("snippet.zig");
    Snippet {
        id: None,
        path: path.clone(),
        language: Language::Zig,
        title: None,
        code: "const sample_binding = @import(\"sample_binding\");\n\npub fn main() void {\n    \
               _ = sample_binding.value();\n}\n"
            .into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path,
            line: 1,
            block_index: 0,
        },
    }
}

/// Regression for the `-I` argument reaching `zig build --build-file`: `zig build` (unlike
/// `zig build-exe`) does not accept `-I` and fails every snippet routed through a real zig
/// package with `unrecognized argument: '-I'`, before compiling a single line. This was a field
/// regression from the `build-exe` include-path fix — its own coverage never drove a project
/// through `zig_package_root`, so it never caught that the include-path application was
/// unconditional. ~keep
#[test]
fn session_include_paths_are_not_forwarded_to_the_build_system_command() {
    if !super::zig_is_runnable() {
        return;
    }
    let (_directory, session) = build_system_project();
    let snippet = build_system_snippet();

    let (status, output) = ZigValidator
        .validate_in_session(
            &snippet,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");

    assert_eq!(
        status,
        SnippetStatus::Pass,
        "a nonempty session.include_paths must not be forwarded as `-I` to `zig build \
         --build-file`, which rejects it outright: {output:?}"
    );
}
