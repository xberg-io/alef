//! Parsing helpers for the `.cwd_relative`-bound options a scaffolded `build.zig` declares --
//! include directories, and (below) the FFI library search directory. Both read back only the
//! shape `scaffold::languages::zig::scaffold_zig` emits; any other expression is skipped rather
//! than guessed at, consistent with the rest of this validator's silence-vs-loudness choices. ~keep

use crate::snippets::error::Result;
use crate::snippets::validators::native_library::{directory_has_ffi_library, linkable_library_names};
use std::path::{Path, PathBuf};

/// Include directories the build manifest declares for its module, in declaration order. ~keep
///
/// A `build.zig` is a program rather than a manifest, so this reads back only the shape Alef's own
/// scaffold (`scaffold::languages::zig::scaffold_zig`) emits:
/// `addIncludePath(.{ .cwd_relative = <expr> })`, where `<expr>` is either a string literal or an
/// identifier [`binding_default`] can trace back to one. Any other expression is skipped rather
/// than guessed at — a wrong `-I` is worse than none.
///
/// Without this the reconstructed `build-exe` command carries no `-I` at all unless the consumer
/// also repeats the path under `include_paths`, so every snippet reaching a `@cInclude` in the
/// binding fails with `C import failed ... 'header.h' not found` while `zig build` succeeds.
/// Paths are returned verbatim, relative to the manifest's own directory — the build root the
/// scaffolded manifest rebases them onto, which the caller supplies.
pub(crate) fn zig_manifest_include_paths(manifest: &Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(manifest)?;
    let mut paths: Vec<String> = Vec::new();
    for expression in path_call_expressions(&source, "addIncludePath") {
        let Some(path) = string_literal(expression)
            .map(str::to_owned)
            .or_else(|| binding_default(&source, expression))
            .or_else(|| binding_default(&source, &format!("{expression}_option")))
        else {
            continue;
        };
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn path_call_expressions<'a>(source: &'a str, method: &str) -> Vec<&'a str> {
    let bare_call = format!("{method}(");
    let receiver_call = format!(".{method}(");
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let arguments = line
                .strip_prefix(&bare_call)
                .or_else(|| {
                    line.find(&receiver_call)
                        .map(|start| &line[start + receiver_call.len()..])
                })?
                .strip_suffix(");")?
                .trim();
            Some(
                arguments
                    .strip_prefix(".{ .cwd_relative = ")
                    .and_then(|expression| expression.strip_suffix(" }"))
                    .unwrap_or(arguments)
                    .trim(),
            )
        })
        .collect()
}

fn string_literal(expression: &str) -> Option<&str> {
    expression.strip_prefix('"')?.strip_suffix('"')
}

/// How many `const` hops [`resolve_binding_statement`] follows before giving up. The scaffold
/// emits exactly one (`ffi_include` → `ffi_include_option`, `ffi_path` → `ffi_path_option`); the
/// rest is headroom, and the bound is what keeps a hand-written manifest with a cyclic binding
/// from spinning here. ~keep
const MAX_BINDING_INDIRECTIONS: usize = 4;

/// Follow a chain of `const <name> = ...;` bindings down to its terminal statement, rebasing
/// through `const <name> = b.pathResolve(&.{ <build root>, <inner> });` when present.
///
/// Shared by [`binding_default`] (which only needs the terminal `orelse` literal) and
/// `zig_manifest_library_path_option` (which also needs the option *name* the same statement
/// declares) — factored out so both read exactly the same chain instead of risking the two
/// silently drifting apart. ~keep
fn resolve_binding_statement(source: &str, name: &str) -> Option<String> {
    const REBASE: &str = "b.pathResolve(&.{";

    let mut name = name.to_owned();
    for _ in 0..MAX_BINDING_INDIRECTIONS {
        let marker = format!("const {name} = ");
        let start = source.find(&marker)? + marker.len();
        let statement = source[start..]
            .split_once(';')
            .map_or(&source[start..], |(head, _)| head);
        let Some(arguments) = statement.strip_prefix(REBASE) else {
            return Some(statement.to_owned());
        };
        let end = arguments.find("})")?;
        name = arguments[..end].rsplit(',').next()?.trim().to_owned();
    }
    None
}

/// The default path a scaffolded `build.zig` binds to `name`.
///
/// Two shapes, both of which alef's own scaffold has emitted:
/// `const <name> = b.option(...) orelse "<default>";`, and the build-root-rebased pair
/// `const <name> = b.pathResolve(&.{ <build root>, <inner> });` whose `<inner>` is itself a
/// binding resolved the same way. The build-root argument is deliberately dropped rather than
/// joined in: the caller already knows that directory as the manifest's own, and it is an
/// build-root expression rather than a literal this could read. ~keep
fn binding_default(source: &str, name: &str) -> Option<String> {
    orelse_literal(&resolve_binding_statement(source, name)?)
}

/// The literal in `... orelse "<default>"` within a single statement.
fn orelse_literal(statement: &str) -> Option<String> {
    const ORELSE: &str = "orelse ";

    let default = statement.find(ORELSE)? + ORELSE.len();
    let literal = statement[default..].trim_start().strip_prefix('"')?;
    let end = literal.find('"')?;
    Some(literal[..end].to_owned())
}

/// The build option *name* in `b.option([]const u8, "<name>", "...") orelse "..."` within a
/// single statement — the string literal that precedes `orelse`, not the description that follows
/// the name. ~keep
fn option_name_in_statement(statement: &str) -> Option<String> {
    let orelse_at = statement.find("orelse ")?;
    let head = &statement[..orelse_at];
    let start = head.find('"')? + 1;
    let end = start + head[start..].find('"')?;
    Some(head[start..end].to_owned())
}

/// The build option name and resolved absolute default directory `addLibraryPath(.{ .cwd_relative
/// = <expr> })` binds, when `<expr>` traces back to a `b.option(...) orelse "<literal>"` — the
/// exact shape alef's own `scaffold_zig` template emits for `ffi_path`. `None` when the manifest
/// carries no such declaration, or when `<expr>` is a bare string literal with no backing option
/// (nothing to override through `-D`/dependency args, so there is nothing this can hand back).
/// ~keep
pub(crate) fn zig_manifest_library_path_option(manifest: &Path) -> Result<Option<(String, PathBuf)>> {
    let source = std::fs::read_to_string(manifest)?;
    let Some(expression) = path_call_expressions(&source, "addLibraryPath").into_iter().next() else {
        return Ok(None);
    };
    let statement = resolve_binding_statement(&source, expression);
    let parsed = statement
        .as_deref()
        .and_then(|statement| Some((option_name_in_statement(statement)?, orelse_literal(statement)?)));
    let parsed = parsed.or_else(|| {
        let statement = resolve_binding_statement(&source, &format!("{expression}_option"))?;
        Some((option_name_in_statement(&statement)?, orelse_literal(&statement)?))
    });
    let Some((option_name, default_literal)) = parsed else {
        return Ok(None);
    };
    let build_root = manifest.parent().unwrap_or(Path::new("."));
    Ok(Some((option_name, build_root.join(default_literal))))
}

/// The library base name in the manifest's first `linkSystemLibrary("<name>", .{})` call.
fn zig_manifest_link_library_name(source: &str) -> Option<String> {
    const DECLARATION: &str = "linkSystemLibrary(\"";

    let start = source.find(DECLARATION)? + DECLARATION.len();
    let end = start + source[start..].find('"')?;
    Some(source[start..end].to_owned())
}

/// The sibling `debug` directory of a `.../release` directory, or `None` when `release_dir`'s own
/// name is not literally `release` — a consumer who points the option somewhere else entirely (a
/// vendored prebuilt, a sibling repo) gets no guessed fallback rather than a wrong one. ~keep
fn sibling_profile_directory(release_dir: &Path) -> Option<PathBuf> {
    (release_dir.file_name()?.to_str()? == "release").then(|| release_dir.with_file_name("debug"))
}

/// Prefer the `release` profile's built FFI library, falling back to `debug` when only that one is
/// on disk.
///
/// `alef build` with no `--release` flag runs a plain `cargo build`, which produces
/// `target/debug/`; the scaffolded `build.zig` only ever searches `target/release/` by default
/// (`scaffold::languages::zig::scaffold_zig`), so every snippet that reaches this manifest fails
/// identically — "unable to find dynamic system library" — whenever the FFI crate was last built
/// without `--release`, even though a perfectly good, symbol-complete library sits one directory
/// over. Returns the option name to override and the absolute directory to set it to; `None` when
/// the release default already resolves (nothing to override) or when neither profile has the
/// library (let zig fail with its own message rather than pass a directory that will not help).
/// ~keep
pub(crate) fn resolve_ffi_library_override(manifest: &Path) -> Result<Option<(String, PathBuf)>> {
    let Some(declared) = declared_ffi_library(manifest)? else {
        return Ok(None);
    };
    match locate_ffi_library(&declared) {
        Some(directory) if directory == declared.release_directory => Ok(None),
        Some(directory) => Ok(Some((declared.option_name, directory))),
        None => Ok(None),
    }
}

/// The FFI library this manifest links against when it is on neither the release nor the debug
/// path -- i.e. the build artifact `alef build` was supposed to produce and did not.
///
/// Reported as the release-profile path the scaffolded `build.zig` searches by default, because
/// that is the one a reader should expect a build to fill. `None` means "nothing to say": either
/// the manifest declares no library search option or no `linkSystemLibrary` (so this file cannot
/// know what to look for), or a library really is on disk. Sharing `locate_ffi_library` with
/// `resolve_ffi_library_override` is what stops "which profile has the library" from being
/// answered two different ways by the override and the preflight. ~keep
///
/// The one shape this could misread, recorded because a false positive costs a corpus its
/// validation: a hand-written `build.zig` whose *first* `linkSystemLibrary` names a genuine system
/// library (`c`, `m`) the linker resolves from its own search path, while also carrying the
/// option-backed `addLibraryPath` this reads. Both conditions must hold — the option-backed
/// library path is the exact construct `scaffold::languages::zig::scaffold_zig` emits to point at
/// the FFI build output, and every `linkSystemLibrary` that scaffold emits names the FFI library
/// itself. A consumer who hits it can still pass `--skip-snippet-validation`. ~keep
pub(crate) fn unresolvable_ffi_library(manifest: &Path) -> Result<Option<PathBuf>> {
    let Some(declared) = declared_ffi_library(manifest)? else {
        return Ok(None);
    };
    if locate_ffi_library(&declared).is_some() {
        return Ok(None);
    }
    let name = linkable_library_names(&declared.library_name)
        .into_iter()
        .next()
        .expect("every host links at least one library name");
    Ok(Some(declared.release_directory.join(name)))
}

/// A manifest's declared FFI library: the `-Doption` name that redirects its search directory, the
/// directory that option defaults to, and the base name it links.
struct DeclaredFfiLibrary {
    option_name: String,
    release_directory: PathBuf,
    library_name: String,
}

fn declared_ffi_library(manifest: &Path) -> Result<Option<DeclaredFfiLibrary>> {
    let Some((option_name, release_directory)) = zig_manifest_library_path_option(manifest)? else {
        return Ok(None);
    };
    let source = std::fs::read_to_string(manifest)?;
    let Some(library_name) = zig_manifest_link_library_name(&source) else {
        return Ok(None);
    };
    Ok(Some(DeclaredFfiLibrary {
        option_name,
        release_directory,
        library_name,
    }))
}

/// The directory that actually holds `declared`'s library: its declared (release) directory when
/// that one has it, otherwise the sibling `debug` directory, otherwise `None`.
fn locate_ffi_library(declared: &DeclaredFfiLibrary) -> Option<PathBuf> {
    if directory_has_ffi_library(&declared.release_directory, &declared.library_name) {
        return Some(declared.release_directory.clone());
    }
    let debug = sibling_profile_directory(&declared.release_directory)?;
    directory_has_ffi_library(&debug, &declared.library_name).then_some(debug)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_build_zig(with_include: bool) -> String {
        let include = if with_include {
            "translate_c.addIncludePath(.{ .cwd_relative = ffi_include });\n"
        } else {
            ""
        };
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const ffi_include = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"vendor/include\";\n\
             \x20   const target = b.standardTargetOptions(.{{}});\n\
             \x20   const optimize = b.standardOptimizeOption(.{{}});\n\
             \x20   const translate_c = b.addTranslateC(.{{\n\
             \x20       .root_source_file = b.path(\"src/c.h\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20   }});\n\
             \x20   {include}\
             \x20   const module = b.addModule(\"sample_binding\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   module.addImport(\"c\", translate_c.createModule());\n\
             }}\n"
        )
    }

    /// The include declaration alef's scaffold emits today. C translation belongs to the build
    /// system because Zig 0.17 removed the `@cImport` builtin. ~keep
    pub(crate) fn build_root_rebased_build_zig(package_name: &str) -> String {
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const target = b.standardTargetOptions(.{{}});\n\
             \x20   const optimize = b.standardOptimizeOption(.{{}});\n\
             \x20   const ffi_include_option = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"vendor/include\";\n\
             \x20   const ffi_include = b.path(ffi_include_option);\n\
             \x20   const translate_c = b.addTranslateC(.{{\n\
             \x20       .root_source_file = b.path(\"src/c.h\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20   }});\n\
             \x20   translate_c.addIncludePath(ffi_include);\n\
             \x20   const module = b.addModule(\"{package_name}\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   module.addImport(\"c\", translate_c.createModule());\n\
             }}\n"
        )
    }

    /// The `ffi_path`/`addLibraryPath` declaration alef's scaffold emits today, mirroring
    /// [`build_root_rebased_build_zig`] but for the library search directory instead of the
    /// include directory -- the shape `resolve_ffi_library_override` reads.
    /// Writes a stand-in FFI library into `directory` under a name this host's zig actually
    /// searches for.
    ///
    /// Hard-coding `lib{name}.dylib` made the tests below pass on Windows for a reason that had
    /// nothing to do with what they assert: the probe could not see that file either, so "no
    /// library was built here" and "the probe cannot recognise a library that is here" rendered
    /// identically. ~keep
    fn write_stand_in_library(directory: &Path, lib_name: &str) {
        std::fs::create_dir_all(directory).unwrap();
        let name = linkable_library_names(lib_name)
            .into_iter()
            .next()
            .expect("every host links at least one library name");
        std::fs::write(directory.join(name), "fake").unwrap();
    }

    pub(crate) fn build_root_rebased_ffi_path_build_zig(lib_name: &str, default_dir: &str) -> String {
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const target = b.standardTargetOptions(.{{}});\n\
             \x20   const optimize = b.standardOptimizeOption(.{{}});\n\
             \x20   const ffi_path_option = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_path\",\n\
             \x20       \"Path to directory containing lib{lib_name}.{{dylib,so,dll,a}}\"\n\
             \x20   ) orelse \"{default_dir}\";\n\
             \x20   const ffi_path: std.Build.LazyPath = if (std.fs.path.isAbsolute(ffi_path_option))\n\
             \x20       .{{ .cwd_relative = ffi_path_option }}\n\
             \x20   else\n\
             \x20       b.path(ffi_path_option);\n\
             \x20   const module = b.addModule(\"sample_binding\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   module.addLibraryPath(ffi_path);\n\
             \x20   module.linkSystemLibrary(\"{lib_name}\", .{{}});\n\
             }}\n"
        )
    }

    /// Alef's own scaffold binds the include directory through a `b.option(...) orelse`
    /// default, so reading only string literals finds nothing in the manifest Alef itself writes.
    #[test]
    fn manifest_include_paths_resolve_through_the_build_option_default() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(true)).unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["vendor/include"]);
    }

    #[test]
    fn manifest_include_paths_accept_a_direct_string_literal() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(
            &manifest,
            "const module = b.addModule(\"sample_binding\", .{\n    .root_source_file = b.path(\"src/root.zig\"),\n});\nmodule.addIncludePath(.{ .cwd_relative = \"include\" });\n",
        )
        .unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["include"]);
    }

    /// Generated build scripts always qualify path methods with a receiver. Pin both the
    /// translated-header and native-library forms so this parser cannot silently return an empty
    /// set after matching only the impossible unqualified shape. ~keep
    #[test]
    fn receiver_qualified_path_calls_return_their_actual_expressions() {
        let source = "translate_c.addIncludePath(ffi_include);\n\
                      module.addLibraryPath(.{ .cwd_relative = ffi_path });\n";

        assert_eq!(path_call_expressions(source, "addIncludePath"), ["ffi_include"]);
        assert_eq!(path_call_expressions(source, "addLibraryPath"), ["ffi_path"]);
    }

    #[test]
    fn a_manifest_without_an_include_declaration_contributes_no_paths() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(false)).unwrap();

        assert!(zig_manifest_include_paths(&manifest).unwrap().is_empty());
    }

    /// Regression: reading only `const <name> = b.option(...) orelse "<literal>"` finds nothing in
    /// the manifest alef writes today, because the binding `.cwd_relative` names is the rebased one.
    #[test]
    fn manifest_include_paths_resolve_through_a_build_root_rebased_binding() {
        let directory = tempfile::tempdir().expect("project directory");
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, build_root_rebased_build_zig("sample_binding")).unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["vendor/include"]);
    }

    #[test]
    fn binding_default_gives_up_rather_than_looping_on_a_cyclic_binding() {
        let source = "const a = b.pathResolve(&.{ root, b_name });\nconst b_name = b.pathResolve(&.{ root, a });\n";

        assert_eq!(binding_default(source, "a"), None);
    }

    /// Regression for the defect this module exists to fix: a manifest whose `ffi_path` default
    /// names a `release` directory that was never built (`alef build` with no `--release` produces
    /// `target/debug/` instead) must resolve to the `debug` sibling that *was* built, carrying the
    /// option name the scaffold declared so the caller can thread it through `b.dependency(...)`.
    #[test]
    fn resolve_ffi_library_override_falls_back_to_debug_when_release_is_missing() {
        let directory = tempfile::tempdir().expect("project directory");
        write_stand_in_library(&directory.path().join("debug"), "sample_ffi");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        let (option_name, resolved) = resolve_ffi_library_override(&manifest).unwrap().unwrap();

        assert_eq!(option_name, "ffi_path");
        // Lexical, not canonicalized -- `zig_manifest_library_path_option` never touches the
        // filesystem to collapse `..`, matching `relative_path`'s own no-canonicalize rationale
        // elsewhere in this module (avoiding a surprise resolution through a symlink). The
        // uncollapsed form still resolves correctly on disk, proven by
        // `a_snippet_links_against_the_debug_profile_when_release_is_missing`'s real `zig build`.
        assert_eq!(resolved, package.join("../debug"));
    }

    /// Negative control for the regression above: when the manifest's own `release` default
    /// already has the library, there is nothing to override -- an eager fix that always redirects
    /// to `debug` would break this case, so it must stay green. Both profiles carry a library here
    /// on purpose: with only `release` built, an always-redirect-to-`debug` bug would coincidentally
    /// still return `None` (nothing at the guessed `debug` sibling either), so it would not catch
    /// the over-eager shape. Building both is what makes "prefers release, does not gratuitously
    /// redirect" an observable difference. ~keep
    #[test]
    fn resolve_ffi_library_override_is_a_no_op_when_release_already_has_the_library() {
        let directory = tempfile::tempdir().expect("project directory");
        write_stand_in_library(&directory.path().join("release"), "sample_ffi");
        write_stand_in_library(&directory.path().join("debug"), "sample_ffi");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(resolve_ffi_library_override(&manifest).unwrap(), None);
    }

    /// A third control: when *neither* profile has been built, there is nothing this can offer --
    /// it must not hand back a debug directory that does not exist either.
    #[test]
    fn resolve_ffi_library_override_is_a_no_op_when_neither_profile_is_built() {
        let directory = tempfile::tempdir().expect("project directory");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(resolve_ffi_library_override(&manifest).unwrap(), None);
    }

    /// The preflight's half of the same question: with neither profile built, the artifact the
    /// manifest links is genuinely absent and must be reported, naming the release path a build
    /// would fill. This is the fact `runner::artifact_preflight` turns into one skip instead of
    /// one doomed `zig build` per snippet. ~keep
    #[test]
    fn an_unbuilt_ffi_library_is_reported_with_the_path_a_build_would_fill() {
        let directory = tempfile::tempdir().expect("project directory");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        let missing = unresolvable_ffi_library(&manifest).unwrap().expect("nothing is built");

        assert!(
            missing.starts_with(package.join("../release")),
            "the report must name the release directory the manifest searches: {}",
            missing.display()
        );
        assert!(
            missing.to_string_lossy().contains("sample_ffi"),
            "the report must name the library: {}",
            missing.display()
        );
    }

    /// The control that keeps the preflight from skipping a session it could have validated: once
    /// the library is on disk, nothing is missing. ~keep
    #[test]
    fn a_built_ffi_library_leaves_nothing_for_the_preflight_to_report() {
        let directory = tempfile::tempdir().expect("project directory");
        write_stand_in_library(&directory.path().join("release"), "sample_ffi");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(unresolvable_ffi_library(&manifest).unwrap(), None);
    }

    /// And the debug-profile control: a library built without `--release` is still a built library,
    /// so a run that would have validated fine must not be skipped. This is the same asymmetry
    /// `resolve_ffi_library_override` exists for, read from the preflight's side. ~keep
    #[test]
    fn a_debug_only_ffi_library_leaves_nothing_for_the_preflight_to_report() {
        let directory = tempfile::tempdir().expect("project directory");
        write_stand_in_library(&directory.path().join("debug"), "sample_ffi");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(unresolvable_ffi_library(&manifest).unwrap(), None);
    }

    #[test]
    fn directory_has_ffi_library_never_credits_a_deps_only_copy() {
        let directory = tempfile::tempdir().expect("project directory");
        write_stand_in_library(&directory.path().join("deps"), "sample_ffi");

        assert!(!directory_has_ffi_library(directory.path(), "sample_ffi"));
    }

    /// Every name in the probe's own candidate set has to be one it can find on disk, or the
    /// override it gates never fires for a library that is really there. Table-driven over the
    /// host's whole set rather than over one representative extension: the Windows defect was a
    /// single wrong member of that set, not a wrong lookup. ~keep
    #[test]
    fn directory_has_ffi_library_finds_every_name_this_host_can_link() {
        for name in linkable_library_names("sample_ffi") {
            let directory = tempfile::tempdir().expect("project directory");
            std::fs::write(directory.path().join(&name), "fake").unwrap();

            assert!(
                directory_has_ffi_library(directory.path(), "sample_ffi"),
                "{name} is a name this host links but the probe does not find"
            );
        }
    }

    /// The defect itself, as the platform states it: Windows dynamic and import libraries carry no
    /// `lib` prefix, so `lib{name}.dll` is a file no toolchain there produces and zig never
    /// searches for -- its own diagnostic names `{name}.dll`, `{name}.lib`, `lib{name}.a`.
    /// Asserting only that `{name}.dll` is found would still pass with the old unconditional
    /// `lib` prefix left in place beside it, and the probe would go on crediting a library that
    /// cannot be linked.
    ///
    /// macOS and Linux get their own branches rather than one combined `else`, because a combined
    /// branch is exactly the bug `native_library::tests` regresses directly: it let a stray
    /// `.dylib` satisfy a Linux probe (and a stray `.so` satisfy a macOS one). This test only
    /// covers whichever platform actually runs it; the exhaustive three-platform table lives in
    /// `native_library::tests`, which does not need `cfg!` to reach the other two. ~keep
    #[test]
    fn the_probe_names_match_what_this_host_actually_produces() {
        let expected: Vec<&str> = if cfg!(target_os = "windows") {
            vec!["sample_ffi.dll", "sample_ffi.lib", "libsample_ffi.a"]
        } else if cfg!(target_os = "macos") {
            vec!["libsample_ffi.dylib", "libsample_ffi.a"]
        } else {
            vec!["libsample_ffi.so", "libsample_ffi.a"]
        };

        assert_eq!(linkable_library_names("sample_ffi"), expected);

        let directory = tempfile::tempdir().expect("project directory");
        // The other platforms' dynamic-library extension: never a name this host's zig searches
        // for, so a copy left by a different platform's build must not count. ~keep
        let never_produced = if cfg!(target_os = "windows") {
            "libsample_ffi.dll"
        } else if cfg!(target_os = "macos") {
            "libsample_ffi.so"
        } else {
            "libsample_ffi.dylib"
        };
        std::fs::write(directory.path().join(never_produced), "fake").unwrap();

        assert!(
            !directory_has_ffi_library(directory.path(), "sample_ffi"),
            "{never_produced} is not a name this host produces or links, so it must not count"
        );
    }
}
