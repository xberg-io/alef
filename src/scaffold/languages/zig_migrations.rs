//! In-place repairs for the create-once artifacts `scaffold_zig` seeds.
//!
//! `packages/zig/build.zig` and `packages/zig/examples/example.zig` are both emitted
//! `generated_header: false`, which `write_scaffold_files_report`'s ownership guard treats as
//! create-only — deliberately, since consumers legitimately hand-edit both. A fix to either
//! template therefore reaches new repos only, and every existing consumer keeps the pre-fix
//! content forever with nothing reporting it. Each repair below matches one known-bad shape and
//! is a strict no-op otherwise, so a consumer edit is never at risk. ~keep

use anyhow::Context as _;
use std::path::Path;

/// Path of the create-once build manifest every migration below repairs, relative to the repo root.
const BUILD_ZIG_RELATIVE: &str = "packages/zig/build.zig";

/// Overwrite `path` with `content` through a same-directory temporary file, so a crash mid-write
/// cannot leave a consumer's hand-edited file truncated. ~keep
fn replace_in_place(path: &Path, content: &str) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, content.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

/// Byte range of the string literal closing the `ffi_include_path` build option's `orelse`, i.e.
/// the include directory a `build.zig` searches when the caller passes no `-Dffi_include_path`.
///
/// Anchored on the option's own *name* rather than on the `const` it binds, so it reads both the
/// historical single-binding shape (`const ffi_include = b.option(...)`) and the current
/// build-root-resolved pair (`const ffi_include_option = ...; const ffi_include = ...`). ~keep
fn ffi_include_default_span(content: &str) -> Option<std::ops::Range<usize>> {
    const OPTION_NAME: &str = "\"ffi_include_path\",";
    const ORELSE_LITERAL: &str = "orelse \"";

    let anchor = content.find(OPTION_NAME)? + OPTION_NAME.len();
    let start = content[anchor..].find(ORELSE_LITERAL)? + anchor + ORELSE_LITERAL.len();
    let end = start + content[start..].find('"')?;
    Some(start..end)
}

/// Whether `value` is the shape [`scaffold_zig`] emitted before it started deriving the FFI crate
/// directory from `[crates.output] ffi`: `../../crates/<crate-name>-ffi/include`, one path segment
/// under `crates/`, guessed from the Rust crate name alone.
fn is_crate_name_derived_include_default(value: &str) -> bool {
    value
        .strip_prefix("../../crates/")
        .and_then(|directory| directory.strip_suffix("-ffi/include"))
        .is_some_and(|crate_directory| !crate_directory.is_empty() && !crate_directory.contains('/'))
}

/// Repair a pre-existing `packages/zig/build.zig` whose `-Dffi_include_path` default still names
/// the FFI crate directory guessed from the Rust crate name instead of the one `[crates.output]
/// ffi` configures — the defect fixed in [`scaffold_zig`] when it switched to
/// `ResolvedCrateConfig::ffi_crate_path`.
///
/// The two disagree whenever the FFI crate directory is not `crates/<crate-name>-ffi`, and the
/// consequence is not cosmetic: every `@cInclude` in the generated binding fails to resolve, so
/// `zig build` and every generated Zig documentation snippet fail with `C import failed` /
/// `'<header>.h' not found`. `build.zig` is a `generated_header: false` seed, so the corrected
/// default never reaches an existing repo through the normal write path — see the ownership-guard
/// note in `cli::pipeline::generate::scaffold`. ~keep
///
/// `generated` is this run's freshly scaffolded `build.zig` content, which is where the corrected
/// default is read from; deriving it here from config a second time would create two producers of
/// one fact that can then disagree.
///
/// A strict no-op unless the on-disk default *both* differs from the generated one *and* still
/// matches the crate-name-derived shape this defect produced. A consumer who deliberately points
/// the option somewhere else — a vendored header tree, an absolute path, a sibling directory —
/// keeps it, exactly as [`migrate_build_zig_test_target`] keeps every line it does not own.
pub(crate) fn migrate_zig_build_ffi_include_default(base_dir: &Path, generated: &str) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, BUILD_ZIG_RELATIVE.as_ref())?;
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let (Some(expected), Some(span)) = (
        ffi_include_default_span(generated).map(|span| &generated[span]),
        ffi_include_default_span(&content),
    ) else {
        return Ok(false);
    };
    let existing = &content[span.clone()];
    if existing == expected || !is_crate_name_derived_include_default(existing) {
        return Ok(false);
    }

    let repaired = format!("{}{expected}{}", &content[..span.start], &content[span.end..]);
    replace_in_place(&path, &repaired)?;
    // Fires only after the replace_in_place above already succeeded: a completed self-heal, not
    // an outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        previous = existing,
        corrected = expected,
        "repaired pre-existing build.zig: -Dffi_include_path default now points at the FFI crate \
         directory [crates.output] ffi configures instead of the one guessed from the crate name"
    );
    Ok(true)
}

/// Repair a pre-existing `packages/zig/build.zig` whose `test_module` still points at the
/// generated `src/<module>.zig` (zero `test` blocks) instead of the seeded
/// `test/<module>_test.zig` — the exact defect fixed in [`scaffold_zig`] above.
///
/// `build.zig` is emitted `generated_header: false` (create-only: see
/// `write_scaffold_files_report`'s ownership guard in
/// `cli::pipeline::generate::scaffold`), and deliberately so — all three consumer trees
/// checked while designing this migration carry direct, hand-written `build.zig` commits,
/// including at least one per tree hand-patching this *exact* `test_module` defect before
/// this migration existed, plus unrelated fixes (a corrected `ffi_include_path` default
/// after a crate rename) that the generator's own template still does not know how to
/// reproduce. A full regenerate-and-overwrite (e.g. by giving the file a self-marker so the
/// normal write path could update it) would silently destroy those. So this repairs only
/// the one known-bad shape, byte-for-byte, and leaves every other line — indentation, added
/// build steps, unrelated hand fixes — untouched.
///
/// Detection is scoped to the `const test_module = b.createModule(.{ ... });` block only
/// (bounded by its own `});`), so the *library* module's identical-looking
/// `.root_source_file = b.path("src/<module>.zig")` line is never a candidate — only the
/// `test_module` one is defective. Silently returns `Ok(false)` (no-op) when: the file does
/// not exist yet (nothing to migrate), the `test_module` block is missing (not this shape at
/// all — a from-scratch hand-authored `build.zig`), or the block's `.root_source_file` no
/// longer matches the known-bad `src/*.zig` pattern (already migrated, or hand-fixed to some
/// other shape) — idempotent by construction, so calling this on every scaffold run is safe.
///
/// Also inserts `test_module.addImport("<module>", module);` immediately after the block's
/// closing `});`, but only when that exact self-import is not already present — the pre-fix
/// template never wired it, so a freshly repointed test module would otherwise fail to
/// resolve `@import("<module>")`. The check is deliberately for the whole call and not for a
/// bare `test_module.addImport(` prefix: a consumer may already import unrelated third-party
/// modules into the test module (one consumer's test module imports a third-party parser
/// module), and a prefix match would read those as the self-import and skip the one line
/// that matters. ~keep
pub(crate) fn migrate_build_zig_test_target(base_dir: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, BUILD_ZIG_RELATIVE.as_ref())?;
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };

    let Some(migrated) = repair_build_zig_test_target(&content) else {
        return Ok(false);
    };
    if migrated == content {
        return Ok(false);
    }

    replace_in_place(&path, &migrated)?;
    // Fires only after the replace_in_place above already succeeded: a completed self-heal, not
    // an outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing build.zig: test_module now points at test/<module>_test.zig \
         instead of the generated src/<module>.zig (zero test blocks)"
    );
    Ok(true)
}

/// Pure line-based transform behind [`migrate_build_zig_test_target`]. Returns `None` when
/// `content` does not contain the known-bad shape at all (no migration candidate); returns
/// `Some(content.to_string())` unchanged when the shape was already repaired by a prior call
/// in the same content (defensive — the caller already short-circuits on this too).
fn repair_build_zig_test_target(content: &str) -> Option<String> {
    const BLOCK_ANCHOR: &str = "const test_module = b.createModule(.{";
    const BAD_PREFIX: &str = ".root_source_file = b.path(\"src/";
    const BAD_SUFFIX: &str = ".zig\"),";

    let lines: Vec<&str> = content.lines().collect();
    let block_start = lines.iter().position(|line| line.trim() == BLOCK_ANCHOR)?;
    let block_end = lines[block_start..]
        .iter()
        .position(|line| line.trim() == "});")
        .map(|offset| block_start + offset)?;

    let bad_line_index = (block_start..block_end).find(|&index| {
        let trimmed = lines[index].trim();
        trimmed.starts_with(BAD_PREFIX) && trimmed.ends_with(BAD_SUFFIX)
    })?;

    let trimmed = lines[bad_line_index].trim();
    let module_name = trimmed.strip_prefix(BAD_PREFIX)?.strip_suffix(BAD_SUFFIX)?;
    // Leading indent only (`trim_start`, not `trim`): `line.len() - line.trim().len()` would
    // count *trailing* whitespace too, and if the matched line ever carried any, that extra
    // width would be byte-sliced off the front here, corrupting the rewritten line. This
    // repair's entire purpose is exact preservation of everything outside the one field it
    // touches, so getting this slice wrong is a real correctness bug, not a style nit. ~keep
    let field_indent = &lines[bad_line_index][..lines[bad_line_index].len() - lines[bad_line_index].trim_start().len()];
    // Statement-level indent (one level shallower than the struct-literal field above),
    // taken from the block's own closing `});` line, for the sibling `test_module.*`
    // call this may insert — not `field_indent`, which belongs to a field one level deeper. ~keep
    let stmt_indent = &lines[block_end][..lines[block_end].len() - lines[block_end].trim_start().len()];

    let mut repaired: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    repaired[bad_line_index] = format!("{field_indent}.root_source_file = b.path(\"test/{module_name}_test.zig\"),");

    let import_call = format!("test_module.addImport(\"{module_name}\", module);");
    if !repaired.iter().any(|line| line.contains(import_call.as_str())) {
        repaired.insert(block_end + 1, format!("{stmt_indent}{import_call}"));
    }

    let mut joined = repaired.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// The exact `examples/example.zig` body [`scaffold_zig`] emitted before the fix that rewrote
/// it for Zig 0.16's `std.Io` API.
const STALE_EXAMPLE_ZIG: &str = "const std = @import(\"std\");\n\npub fn main() !void {\n    var gpa = std.heap.GeneralPurposeAllocator(.{}){};\n    defer _ = gpa.deinit();\n    const allocator = gpa.allocator();\n\n    const stdout = std.io.getStdOut().writer();\n    try stdout.print(\"Example: module loaded successfully\\n\", .{});\n}\n";

/// Repair a pre-existing `packages/zig/examples/example.zig` that still carries the
/// pre-Zig-0.16 example -- the exact defect fixed when `scaffold_zig`'s `example_zig` literal
/// was rewritten to Zig 0.16's `std.Io` API (`cc7f824b0`, "update Zig example for 0.16").
///
/// `examples/example.zig` is `generated_header: false` (create-only), so a repo scaffolded
/// before that fix keeps shipping an example that no longer compiles under the pinned Zig
/// 0.16 toolchain forever: `std.heap.GeneralPurposeAllocator`/`std.io.getStdOut` were removed,
/// and the unused `allocator` binding is a hard error too. Every scaffolded Zig package gets
/// the identical literal (no crate name, no module name — this file has no per-project
/// variables at all), so an exact byte match against the one known-bad constant is both
/// sufficient and maximally conservative: any consumer edit at all — even just adding a
/// comment — fails the match and leaves the file completely untouched. ~keep
pub(crate) fn migrate_zig_example(base_dir: &Path, relative_path: &Path, replacement: &str) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if existing != STALE_EXAMPLE_ZIG {
        return Ok(false);
    }
    if existing == replacement {
        return Ok(false);
    }

    replace_in_place(&path, replacement)?;
    // Fires only after the replace_in_place above already succeeded: a completed self-heal, not
    // an outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing packages/zig/examples/example.zig: replaced the pre-Zig-0.16 \
         example (std.heap.GeneralPurposeAllocator/std.io.getStdOut) with the \
         std.Io.Threaded-based one"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
    use crate::core::ir::ApiSurface;
    use std::path::PathBuf;

    /// The `build.zig` this run would scaffold from scratch, which is where
    /// [`migrate_zig_build_ffi_include_default`] reads its corrected default from.
    fn freshly_generated_build_zig() -> String {
        let config: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
        .expect("valid config");
        let resolved: ResolvedCrateConfig = config.resolve().expect("resolve").remove(0);
        super::super::zig::scaffold_zig(&ApiSurface::default(), &resolved)
            .expect("scaffold")
            .into_iter()
            .find(|file| file.path == std::path::Path::new("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content
    }

    /// A representative pre-fix `build.zig`: the library `module` and the `test_module`
    /// both point at `src/my_lib.zig` (the known-bad shape), and the FFI include default was
    /// hand-fixed after a crate rename — exactly the kind of hand edit found in every real
    /// consumer repo (liter-llm, tree-sitter-language-pack, html-to-markdown) this migration
    /// was designed against.
    fn known_bad_build_zig() -> String {
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const ffi_path = b.option(
        []const u8,
        "ffi_path",
        "Path to directory containing libmy_lib_ffi.{dylib,so,dll,a}"
    ) orelse "../../target/release";

    const ffi_include = b.option(
        []const u8,
        "ffi_include_path",
        "Path to directory containing the FFI C header"
    ) orelse "../../crates/my-lib-ffi/include"; // hand-fixed after a crate rename

    const module = b.addModule("my_lib", .{
        .root_source_file = b.path("src/my_lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    module.addLibraryPath(.{ .cwd_relative = ffi_path });
    module.addIncludePath(.{ .cwd_relative = ffi_include });
    module.linkSystemLibrary("my_lib_ffi", .{});

    const test_module = b.createModule(.{
        .root_source_file = b.path("src/my_lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    test_module.addLibraryPath(.{ .cwd_relative = ffi_path });
    test_module.addIncludePath(.{ .cwd_relative = ffi_include });
    test_module.linkSystemLibrary("my_lib_ffi", .{});

    const tests = b.addTest(.{
        .root_module = test_module,
    });

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
}
"#
        .to_string()
    }

    /// The core repair: only the `test_module` block's `.root_source_file` is repointed at
    /// `test/my_lib_test.zig`, and the missing `addImport` wiring is inserted right after
    /// that block — while the identical-looking line in the *library* `module` block, and
    /// every hand-written line (the crate-rename `ffi_include_path` fix), survive verbatim.
    #[test]
    fn repairs_only_the_test_module_target() {
        let original = known_bad_build_zig();
        let repaired = repair_build_zig_test_target(&original).expect("known-bad shape must match");

        assert!(
            repaired.contains(".root_source_file = b.path(\"test/my_lib_test.zig\"),"),
            "got:\n{repaired}"
        );
        assert!(
            repaired.contains("test_module.addImport(\"my_lib\", module);"),
            "got:\n{repaired}"
        );
        // The library module's own identical-looking line must be untouched.
        assert!(
            repaired.contains(
                "const module = b.addModule(\"my_lib\", .{\n        .root_source_file = b.path(\"src/my_lib.zig\"),"
            ),
            "library module's root_source_file must not be touched, got:\n{repaired}"
        );
        // The hand-fixed ffi_include_path default must survive byte-for-byte.
        assert!(
            repaired.contains("orelse \"../../crates/my-lib-ffi/include\"; // hand-fixed after a crate rename"),
            "hand edit must survive untouched, got:\n{repaired}"
        );
        // Only one line changed in the test_module block itself, plus one inserted line —
        // every other line of the 50-odd-line file is byte-identical.
        let original_lines: std::collections::HashSet<&str> = original.lines().collect();
        let unexpected_new_lines: Vec<&str> = repaired.lines().filter(|line| !original_lines.contains(line)).collect();
        assert_eq!(
            unexpected_new_lines,
            vec![
                "        .root_source_file = b.path(\"test/my_lib_test.zig\"),",
                "    test_module.addImport(\"my_lib\", module);",
            ],
            "no line beyond the two expected repairs should differ, got:\n{repaired}"
        );
    }

    /// Idempotent: running the repair against its own output finds nothing left to fix.
    #[test]
    fn is_a_no_op_once_already_repaired() {
        let repaired_once = repair_build_zig_test_target(&known_bad_build_zig()).expect("first repair applies");
        assert!(
            repair_build_zig_test_target(&repaired_once).is_none(),
            "a second pass over already-repaired content must be a no-op"
        );
    }

    /// A `build.zig` with no recognizable `test_module` block at all (a from-scratch,
    /// hand-authored file) is not a migration candidate and must be left alone.
    #[test]
    fn ignores_a_build_zig_without_a_test_module_block() {
        let custom = "const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n    _ = b;\n}\n";
        assert!(repair_build_zig_test_target(custom).is_none());
    }

    /// The current (already-fixed) generator output already points `test_module` at
    /// `test/<module>_test.zig` — this must not match the known-bad pattern and must be
    /// left alone, since re-touching an already-correct file on every run would defeat the
    /// whole point of the guard being idempotent.
    #[test]
    fn ignores_a_build_zig_already_pointing_at_the_test_dir() {
        let already_fixed = known_bad_build_zig().replacen(
            ".root_source_file = b.path(\"src/my_lib.zig\"),\n        .target = target,\n        .optimize = optimize,\n        .link_libc = true,\n    });\n    test_module.addLibraryPath",
            ".root_source_file = b.path(\"test/my_lib_test.zig\"),\n        .target = target,\n        .optimize = optimize,\n        .link_libc = true,\n    });\n    test_module.addImport(\"my_lib\", module);\n    test_module.addLibraryPath",
            1,
        );
        assert!(repair_build_zig_test_target(&already_fixed).is_none());
    }

    /// End-to-end control via [`migrate_build_zig_test_target`]: writes a known-bad
    /// `build.zig` carrying a genuine hand edit (the crate-rename `ffi_include_path` fix) to
    /// a tempdir, runs the migration, and proves the hand edit is still present afterward —
    /// the exact guarantee this migration exists to provide instead of a blind overwrite.
    #[test]
    fn migrates_on_disk_and_preserves_a_hand_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let zig_dir = dir.path().join("packages/zig");
        std::fs::create_dir_all(&zig_dir).expect("create packages/zig");
        std::fs::write(zig_dir.join("build.zig"), known_bad_build_zig()).expect("write build.zig");

        let changed = migrate_build_zig_test_target(dir.path()).expect("migration must not error");
        assert!(changed, "known-bad file must be reported as changed");

        let on_disk = std::fs::read_to_string(zig_dir.join("build.zig")).expect("read migrated build.zig");
        assert!(on_disk.contains(".root_source_file = b.path(\"test/my_lib_test.zig\"),"));
        assert!(
            on_disk.contains("orelse \"../../crates/my-lib-ffi/include\"; // hand-fixed after a crate rename"),
            "hand-fixed ffi_include_path default must survive the on-disk migration, got:\n{on_disk}"
        );

        // Running it again against the now-repaired file must be a no-op.
        let changed_again = migrate_build_zig_test_target(dir.path()).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-repaired file must be a no-op"
        );
    }

    /// A `build.zig` that does not exist yet (nothing scaffolded so far) must not be
    /// created or error — there is nothing to migrate.
    #[test]
    fn is_a_no_op_when_build_zig_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let changed = migrate_build_zig_test_target(dir.path()).expect("must not error on a missing file");
        assert!(!changed);
        assert!(!dir.path().join("packages/zig/build.zig").exists());
    }

    const FIXED_EXAMPLE_ZIG: &str = "const std = @import(\"std\");\n\npub fn main() !void {\n    var threaded: std.Io.Threaded = .init(std.heap.smp_allocator, .{});\n    defer threaded.deinit();\n\n    var stdout_buffer: [64]u8 = undefined;\n    var stdout_writer = std.Io.File.stdout().writer(threaded.io(), &stdout_buffer);\n    const stdout = &stdout_writer.interface;\n\n    try stdout.print(\"Example: module loaded successfully\\n\", .{});\n    try stdout.flush();\n}\n";

    #[test]
    fn should_replace_stale_example_zig_predating_the_zig_0_16_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let examples_dir = dir.path().join("packages/zig/examples");
        std::fs::create_dir_all(&examples_dir).expect("create packages/zig/examples");
        std::fs::write(examples_dir.join("example.zig"), STALE_EXAMPLE_ZIG).expect("write stale example.zig");

        let relative_path = Path::new("packages/zig/examples/example.zig");
        let changed =
            migrate_zig_example(dir.path(), relative_path, FIXED_EXAMPLE_ZIG).expect("migration must not error");
        assert!(changed, "the known-stale example.zig must be reported as changed");

        let on_disk = std::fs::read_to_string(examples_dir.join("example.zig")).expect("read migrated file");
        assert_eq!(on_disk, FIXED_EXAMPLE_ZIG);
        assert!(
            !on_disk.contains("GeneralPurposeAllocator") && !on_disk.contains("getStdOut"),
            "removed Zig 0.16 APIs must not remain"
        );

        let changed_again =
            migrate_zig_example(dir.path(), relative_path, FIXED_EXAMPLE_ZIG).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_not_touch_a_hand_edited_example_zig() {
        let dir = tempfile::tempdir().expect("tempdir");
        let examples_dir = dir.path().join("packages/zig/examples");
        std::fs::create_dir_all(&examples_dir).expect("create packages/zig/examples");
        let hand_written =
            "const std = @import(\"std\");\n\npub fn main() !void {\n    std.debug.print(\"hello\\n\", .{});\n}\n";
        std::fs::write(examples_dir.join("example.zig"), hand_written).expect("write hand-edited example.zig");

        let relative_path = Path::new("packages/zig/examples/example.zig");
        let changed =
            migrate_zig_example(dir.path(), relative_path, FIXED_EXAMPLE_ZIG).expect("migration must not error");
        assert!(!changed, "a hand-edited example.zig must never be touched");

        let on_disk = std::fs::read_to_string(examples_dir.join("example.zig")).expect("read file");
        assert_eq!(
            on_disk, hand_written,
            "hand-edited example.zig must survive byte-for-byte"
        );
    }

    #[test]
    fn migrate_zig_example_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = Path::new("packages/zig/examples/example.zig");
        let changed = migrate_zig_example(dir.path(), relative_path, FIXED_EXAMPLE_ZIG).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }

    /// A `build.zig` seeded by the pre-fix template: the include default is the
    /// `crates/<crate-name>-ffi` directory guessed from the Rust crate name, which is not where
    /// `[crates.output] ffi` puts the FFI crate. Carries a hand-added build step so the
    /// byte-preservation assertions below have something real to protect.
    fn crate_name_derived_build_zig(include_default: &str) -> String {
        format!(
            "const std = @import(\"std\");\n\
             \n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const ffi_include = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"{include_default}\";\n\
             \x20   const module = b.addModule(\"my_lib\", .{{\n\
             \x20       .root_source_file = b.path(\"src/my_lib.zig\"),\n\
             \x20   }});\n\
             \x20   module.addIncludePath(.{{ .cwd_relative = ffi_include }});\n\
             \x20   _ = b.step(\"docs\", \"a hand-added step alef never emitted\");\n\
             }}\n"
        )
    }

    fn write_build_zig(base_dir: &Path, content: &str) -> PathBuf {
        let path = base_dir.join(BUILD_ZIG_RELATIVE);
        std::fs::create_dir_all(path.parent().expect("build.zig has a parent")).expect("create packages/zig");
        std::fs::write(&path, content).expect("write build.zig");
        path
    }

    /// Regression for the defect that failed every generated Zig snippet in a consumer repo whose
    /// FFI crate directory is not `crates/<crate-name>-ffi`: the create-once `build.zig` keeps
    /// searching the guessed directory forever, so the binding's `@cInclude` never resolves and
    /// `zig build` reports `C import failed` / `'<header>.h' not found`. ~keep
    #[test]
    fn migration_repairs_an_include_default_guessed_from_the_crate_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = crate_name_derived_build_zig("../../crates/my-lib-rs-ffi/include");
        let path = write_build_zig(dir.path(), &stale);
        let generated = crate_name_derived_build_zig("../../crates/my-lib-ffi/include");

        let changed = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");

        assert!(changed);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read build.zig"),
            generated,
            "only the include default may change; every other line stays byte-identical"
        );
    }

    #[test]
    fn migration_is_idempotent_once_the_include_default_is_correct() {
        let dir = tempfile::tempdir().expect("tempdir");
        let generated = crate_name_derived_build_zig("../../crates/my-lib-ffi/include");
        write_build_zig(
            dir.path(),
            &crate_name_derived_build_zig("../../crates/my-lib-rs-ffi/include"),
        );

        let first = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");
        let second = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");

        assert!(first);
        assert!(!second, "a second pass must report no change");
    }

    /// A consumer who deliberately points `-Dffi_include_path` at something other than a
    /// `crates/<name>-ffi/include` directory has chosen a layout alef cannot second-guess, so the
    /// migration must leave it alone even though it disagrees with the generated default.
    #[test]
    fn migration_keeps_an_include_default_the_consumer_repointed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hand_edited = crate_name_derived_build_zig("../../vendor/headers");
        let path = write_build_zig(dir.path(), &hand_edited);
        let generated = crate_name_derived_build_zig("../../crates/my-lib-ffi/include");

        let changed = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");

        assert!(!changed);
        assert_eq!(std::fs::read_to_string(&path).expect("read build.zig"), hand_edited);
    }

    #[test]
    fn migration_is_a_no_op_when_build_zig_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let generated = crate_name_derived_build_zig("../../crates/my-lib-ffi/include");

        let changed = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");

        assert!(!changed);
        assert!(!dir.path().join(BUILD_ZIG_RELATIVE).exists());
    }

    /// The migration reads its corrected value out of the run's freshly generated `build.zig`, so
    /// it must find the default in the shape [`scaffold_zig`] emits today — the build-root-rebased
    /// pair, where the `orelse` literal no longer sits on the binding the module actually uses. ~keep
    #[test]
    fn migration_reads_the_corrected_default_out_of_freshly_generated_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_build_zig(
            dir.path(),
            &crate_name_derived_build_zig("../../crates/my-lib-rs-ffi/include"),
        );
        let generated = freshly_generated_build_zig();

        let changed = migrate_zig_build_ffi_include_default(dir.path(), &generated).expect("must not error");

        assert!(changed);
        assert!(
            std::fs::read_to_string(&path)
                .expect("read build.zig")
                .contains(") orelse \"../../crates/my-lib-ffi/include\";"),
            "the default must come from the generated content, not be re-derived here"
        );
    }
}
