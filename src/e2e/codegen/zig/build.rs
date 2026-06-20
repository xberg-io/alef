use super::*;

pub(super) fn render_build_zig_zon(
    pkg_name: &str,
    pkg_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    version: &str,
    platform_hashes: &BTreeMap<String, (String, Option<String>)>,
    hash_is_stale: bool,
    capsule_deps: &[(String, String, String)],
) -> String {
    let dep_block = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            let use_platform_registry_deps = uses_platform_registry_deps(platform_hashes);
            let mut entries = Vec::new();
            for (platform, (url, hash_opt)) in platform_hashes {
                let dep_name = if use_platform_registry_deps {
                    format!("{}_{}", pkg_name, platform.replace('-', "_"))
                } else {
                    pkg_name.to_string()
                };
                let lazy_line = if use_platform_registry_deps {
                    "\n            .lazy = true,"
                } else {
                    ""
                };
                let entry = match hash_opt {
                    Some(h) if hash_is_stale => {
                        format!(
                            "        // STALE hash (embedded version != current); regenerate via `alef sync-versions`\n        // expected to match crate v{version}, was: {h}\n        .{dep_name} = .{{\n            .url = \"{url}\",{lazy_line}\n        }},",
                            version = version,
                            url = url,
                            h = h,
                            dep_name = dep_name,
                            lazy_line = lazy_line
                        )
                    }
                    Some(h) => {
                        format!(
                            "        .{dep_name} = .{{\n            .url = \"{url}\",\n            .hash = \"{h}\",{lazy_line}\n        }},",
                            dep_name = dep_name,
                            url = url,
                            h = h,
                            lazy_line = lazy_line
                        )
                    }
                    None => {
                        format!(
                            "        .{dep_name} = .{{\n            .url = \"{url}\",{lazy_line}\n        }},",
                            dep_name = dep_name,
                            url = url,
                            lazy_line = lazy_line
                        )
                    }
                };
                entries.push(entry);
            }
            entries.join("\n")
        }
        crate::e2e::config::DependencyMode::Local => {
            // Zig 0.16+ requires named dependencies. Use the package name as the key.
            // Local mode rebuilds the binding module from source (see `build.zig`), so any
            // host-capsule dependency (e.g. zig-tree-sitter) the binding `@import`s must be
            // declared here too — the published-package zon's copy is not consulted.
            let mut block = format!("        .{pkg_name} = .{{\n            .path = \"{pkg_path}\",\n        }},");
            for (module_name, url, hash) in capsule_deps {
                let hash_field = if hash.is_empty() {
                    String::new()
                } else {
                    format!("\n            .hash = \"{hash}\",")
                };
                let _ = write!(
                    block,
                    "\n        .{module_name} = .{{\n            .url = \"{url}\",{hash_field}\n        }},"
                );
            }
            block
        }
    };

    let min_zig = toolchain::MIN_ZIG_VERSION;
    // Zig 0.16+ requires a fingerprint of the form (crc32_ieee(name) << 32) | id.
    let name_bytes: &[u8] = b"e2e_zig";
    let mut crc: u32 = 0xffff_ffff;
    for byte in name_bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    let name_crc: u32 = !crc;
    let mut id: u32 = 0x811c_9dc5;
    for byte in name_bytes {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    let fingerprint: u64 = ((name_crc as u64) << 32) | (id as u64);

    let dep_content = format!(".{{\n{dep_block}\n    }}");

    format!(
        r#".{{
    .name = .e2e_zig,
    .version = "0.1.0",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = {dep_content},
    .paths = .{{
        "build.zig",
        "build.zig.zon",
        "src",
    }},
}}
"#
    )
}

/// Fixture-shape flags that toggle optional `build.zig` wiring.
#[derive(Debug, Clone, Copy)]
pub(super) struct ZigBuildFlags {
    /// Any fixture loads files by path (`file_path`/`bytes` args) and so the
    /// test run step must `setCwd` into the test-documents directory.
    pub(super) has_file_fixtures: bool,
    /// Any fixture hits the mock server, so `build.zig` must spawn it and export
    /// `MOCK_SERVER_URL` into the test run steps.
    pub(super) needs_mock_server: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_build_zig(
    test_filenames: &[String],
    pkg_name: &str,
    module_name: &str,
    ffi_lib_name: &str,
    ffi_crate_path: &str,
    flags: ZigBuildFlags,
    test_documents_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    use_platform_registry_deps: bool,
    env: &std::collections::HashMap<String, String>,
    capsule_deps: &[(String, String, String)],
) -> String {
    let ZigBuildFlags {
        has_file_fixtures,
        needs_mock_server,
    } = flags;
    if test_filenames.is_empty() {
        return match dep_mode {
            crate::e2e::config::DependencyMode::Registry => {
                if !use_platform_registry_deps {
                    return format!(
                        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const {module_name}_module = b.dependency("{pkg_name}", .{{
        .target = target,
        .optimize = optimize,
    }}).module("{module_name}");

    const test_step = b.step("test", "Run tests");
}}
"#
                    );
                }
                format!(
                    r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Fetch the published Zig package from the registry (multi-target lazy dependency).
    const target_os = target.result.os.tag;
    const target_arch = target.result.cpu.arch;

    const {pkg_name}_dep_name = if (target_os == .linux and target_arch == .x86_64)
        "{pkg_name}_x86_64_unknown_linux_gnu"
    else if (target_os == .linux and target_arch == .aarch64)
        "{pkg_name}_aarch64_unknown_linux_gnu"
    else if (target_os == .macos and target_arch == .aarch64)
        "{pkg_name}_aarch64_apple_darwin"
    else if (target_os == .macos and target_arch == .x86_64)
        "{pkg_name}_x86_64_apple_darwin"
    else if (target_os == .windows and target_arch == .x86_64)
        "{pkg_name}_x86_64_pc_windows_msvc"
    else
        @panic("unsupported target — supported: linux-{{x86_64,aarch64}}, macos-{{arm64,x86_64}}, windows-x86_64");

    const {module_name}_module = (b.lazyDependency({pkg_name}_dep_name, .{{
        .target = target,
        .optimize = optimize,
    }}) orelse return).module("{module_name}");

    const test_step = b.step("test", "Run tests");
}}
"#
                )
            }
            crate::e2e::config::DependencyMode::Local => r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run tests");
}
"#
            .to_string(),
        };
    }

    // The Zig build script wires up three names that all derive from the
    // crate config:
    //   * `ffi_lib_name`     — the dynamic library to link (e.g. `mylib_ffi`).
    //   * `pkg_name`         — the Zig package directory and source file stem
    //                          under `packages/zig/src/<pkg_name>.zig`.
    //   * `module_name`      — the Zig `@import("...")` identifier other test
    //                          files use to import the binding module.
    // Callers pass these in resolved form so this function never embeds a
    // binding crate's name.
    let mut content = String::from(
        "const std = @import(\"std\");\nconst builtin = @import(\"builtin\");\n\npub fn build(b: *std.Build) void {\n",
    );
    content.push_str("    const target = b.standardTargetOptions(.{});\n");
    content.push_str("    const optimize = b.standardOptimizeOption(.{});\n");
    content.push_str("    const test_step = b.step(\"test\", \"Run tests\");\n");
    match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            if !use_platform_registry_deps {
                content.push_str("\n    // Fetch the published Zig package from the registry.\n");
                let _ = writeln!(content, "    const {pkg_name}_dep = b.dependency(\"{pkg_name}\", .{{");
                content.push_str("        .target = target,\n");
                content.push_str("        .optimize = optimize,\n");
                let _ = writeln!(content, "    }});");
                let _ = writeln!(
                    content,
                    "    const {module_name}_module = {pkg_name}_dep.module(\"{module_name}\");"
                );
                let _ = writeln!(content, "    const {pkg_name}_lib_path = {pkg_name}_dep.path(\"lib\");");
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_include_path = {pkg_name}_dep.path(\"include\");"
                );
                let _ = writeln!(content, "    {module_name}_module.addLibraryPath({pkg_name}_lib_path);");
                let _ = writeln!(
                    content,
                    "    {module_name}_module.addIncludePath({pkg_name}_include_path);"
                );
                let _ = writeln!(
                    content,
                    "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
                );
                let _ = writeln!(content);
            } else {
                // Registry mode with per-platform assets: use multi-target lazy dependencies (Zig 0.13+).
                // Each platform variant is declared with .lazy = true so Zig only fetches
                // the one matching this build's target triple. The build script selects the
                // right dependency name based on the target via `b.lazyDependency(name, .{})`.
                content.push_str(
                    "\n    // Fetch the published Zig package from the registry (multi-target lazy dependency).\n",
                );
                content.push_str("    // Select the appropriate platform variant based on the target triple.\n");
                content.push_str("    const target_os = target.result.os.tag;\n");
                content.push_str("    const target_arch = target.result.cpu.arch;\n");
                content.push('\n');
                content.push_str(&format!(
                    "    const {pkg_name}_dep_name = if (target_os == .linux and target_arch == .x86_64)\n"
                ));
                content.push_str(&format!("        \"{pkg_name}_x86_64_unknown_linux_gnu\"\n"));
                content.push_str("    else if (target_os == .linux and target_arch == .aarch64)\n");
                content.push_str(&format!("        \"{pkg_name}_aarch64_unknown_linux_gnu\"\n"));
                content.push_str("    else if (target_os == .macos and target_arch == .aarch64)\n");
                content.push_str(&format!("        \"{pkg_name}_aarch64_apple_darwin\"\n"));
                content.push_str("    else if (target_os == .macos and target_arch == .x86_64)\n");
                content.push_str(&format!("        \"{pkg_name}_x86_64_apple_darwin\"\n"));
                content.push_str("    else if (target_os == .windows and target_arch == .x86_64)\n");
                content.push_str(&format!("        \"{pkg_name}_x86_64_pc_windows_msvc\"\n"));
                content.push_str("    else\n");
                content.push_str("        @panic(\"unsupported target — supported: linux-{x86_64,aarch64}, macos-{arm64,x86_64}, windows-x86_64\");\n");
                content.push('\n');
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_dep = b.lazyDependency({pkg_name}_dep_name, .{{"
                );
                content.push_str("        .target = target,\n");
                content.push_str("        .optimize = optimize,\n");
                let _ = writeln!(content, "    }}) orelse return;");
                let _ = writeln!(
                    content,
                    "    const {module_name}_module = {pkg_name}_dep.module(\"{module_name}\");"
                );
                // Conditionally link FFI from the fetched package's bundled lib/include.
                // If the fetched package's build.zig is the new distributable version,
                // it already exports a module with FFI linked, and these lines are
                // redundant but harmless. If the fetched package's build.zig is an old
                // development version (still references ../../target/release), these
                // lines ensure FFI linking works from the tarball's own lib/ directory.
                let _ = writeln!(content, "    const {pkg_name}_lib_path = {pkg_name}_dep.path(\"lib\");");
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_include_path = {pkg_name}_dep.path(\"include\");"
                );
                let _ = writeln!(content, "    {module_name}_module.addLibraryPath({pkg_name}_lib_path);");
                let _ = writeln!(
                    content,
                    "    {module_name}_module.addIncludePath({pkg_name}_include_path);"
                );
                let _ = writeln!(
                    content,
                    "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
                );
                let _ = writeln!(content);
            }
        }
        crate::e2e::config::DependencyMode::Local => {
            let _ = writeln!(
                content,
                "    const ffi_path = b.option([]const u8, \"ffi_path\", \"Path to directory containing lib{ffi_lib_name}\") orelse \"../../target/release\";"
            );
            let _ = writeln!(
                content,
                "    const ffi_include = b.option([]const u8, \"ffi_include_path\", \"Path to directory containing FFI header\") orelse \"{ffi_crate_path}/include\";"
            );
            // Compute absolute FFI path for rpath declarations so dylib loading works
            // regardless of the test binary's working directory (e.g., when chdir'd into test_documents).
            let _ = writeln!(content, "    const ffi_path_abs = b.pathFromRoot(ffi_path);");
            let _ = writeln!(content);
            let _ = writeln!(
                content,
                "    const {module_name}_module = b.addModule(\"{module_name}\", .{{"
            );
            let _ = writeln!(
                content,
                "        .root_source_file = b.path(\"../../packages/zig/src/{module_name}.zig\"),"
            );
            content.push_str("        .target = target,\n");
            content.push_str("        .optimize = optimize,\n");
            // Zig 0.16 requires explicit libc linking for any module that transitively
            // references stdlib C bindings (e.g. `c.getenv` via std.posix). The shared
            // binding module pulls in the FFI header, so libc is always required.
            content.push_str("        .link_libc = true,\n");
            content.push_str("    });\n");
            let _ = writeln!(
                content,
                "    {module_name}_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});"
            );
            let _ = writeln!(
                content,
                "    {module_name}_module.addIncludePath(.{{ .cwd_relative = ffi_include }});"
            );
            let _ = writeln!(
                content,
                "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
            );
            // Add rpath support for macOS dylib runtime linking using the absolute path.
            let _ = writeln!(
                content,
                "    {module_name}_module.addRPath(.{{ .cwd_relative = ffi_path_abs }});"
            );
            // Host-capsule passthrough: Local mode rebuilds the binding module from source,
            // so it must receive the same `tree_sitter` (host-capsule) import the published
            // package's build.zig wires in. The dependency itself is declared in build.zig.zon.
            for (capsule_module, _url, _hash) in capsule_deps {
                let _ = writeln!(
                    content,
                    "    const {capsule_module}_dep = b.dependency(\"{capsule_module}\", .{{ .target = target, .optimize = optimize }});"
                );
                let _ = writeln!(
                    content,
                    "    {module_name}_module.addImport(\"{capsule_module}\", {capsule_module}_dep.module(\"{capsule_module}\"));"
                );
            }
            let _ = writeln!(content);
        }
    }

    // Spawn the mock-server at configure time and capture its ephemeral URL so
    // every test run step can read it via `MOCK_SERVER_URL`. Zig has no
    // test-suite init hook (unlike Go's TestMain or the Python conftest), so the
    // build script itself owns the server's lifetime: it lives as long as the
    // `zig build` process, which spans test execution. A pre-set
    // `MOCK_SERVER_URL` (external CI orchestration) short-circuits the spawn.
    if needs_mock_server {
        content.push_str(render_zig_mock_server_spawn());
        let _ = writeln!(content);
    }

    let mut prev_run: Option<String> = None;
    for filename in test_filenames {
        // Convert filename like "basic_test.zig" to a test name
        let test_name = filename.trim_end_matches("_test.zig");
        content.push_str(&format!("    const {test_name}_module = b.createModule(.{{\n"));
        content.push_str(&format!("        .root_source_file = b.path(\"src/{filename}\"),\n"));
        content.push_str("        .target = target,\n");
        content.push_str("        .optimize = optimize,\n");
        // Each test module also needs libc linking because it imports the binding
        // module (which references C stdlib symbols) and may directly call helpers
        // like `std.c.getenv` for env-var-driven mock-server URLs.
        content.push_str("        .link_libc = true,\n");
        content.push_str("    });\n");
        content.push_str(&format!(
            "    {test_name}_module.addImport(\"{module_name}\", {module_name}_module);\n"
        ));
        // Zig 0.16: addTest hashes its output binary path off the artifact `.name`.
        // Without an explicit name, every addTest call defaults to "test", colliding
        // in the cache — only one binary survives, every other addRunArtifact fails
        // with FileNotFound at its computed path. Setting a unique name per test
        // module produces a distinct .zig-cache/o/<hash>/<name> binary for each.
        //
        // Zig 0.16 ALSO defaults to the self-hosted backend on aarch64-linux for
        // Debug builds. That backend emits the test binary at a different cache
        // path (or with different permissions) than the build system's RunStep
        // computes when reading `getEmittedBin()`, so every `addRunArtifact` call
        // fails with `FileNotFound` at `.zig-cache/o/<hash>/<name>` even though
        // the compile step reports success. Forcing `.use_llvm = true` pins the
        // LLVM backend, which keeps the emitted binary at the path the RunStep
        // expects. Other Zig backends (x86_64 macOS/Linux) already default to
        // LLVM, so this is a no-op there.
        content.push_str(&format!("    const {test_name}_tests = b.addTest(.{{\n"));
        content.push_str(&format!("        .name = \"{test_name}_test\",\n"));
        content.push_str(&format!("        .root_module = {test_name}_module,\n"));
        content.push_str("        .use_llvm = true,\n");
        content.push_str("    });\n");
        // Add rpath support for macOS dylib runtime linking in test artifacts (Local mode only).
        // The test binary itself needs an rpath in its load commands to locate the FFI dylib when run.
        if matches!(dep_mode, crate::e2e::config::DependencyMode::Local) {
            content.push_str(&format!(
                "    {test_name}_tests.root_module.addRPath(.{{ .cwd_relative = ffi_path_abs }});\n"
            ));
        }
        // Run the test binary via `addRunArtifact`. When any fixture reads
        // files from `test_documents/` (arg type `file_path` or `bytes`),
        // also point the working directory at the repo-root `test_documents/`
        // so that `std.Io.Dir.cwd().readFileAlloc(...)` resolves paths like
        // `pdf/fake_memo.pdf` correctly. Other languages perform this chdir
        // in a per-suite hook (Go `TestMain`, Python conftest, Kotlin Gradle
        // `workingDir`); Zig has no equivalent test-suite init hook, so it
        // must happen at the build-step level.
        //
        // IMPORTANT: `setCwd` is only emitted when `has_file_fixtures` is
        // true. For consumers whose fixtures are mock-server-only, there is
        // no `test_documents/` directory. Zig's
        // RunStep chdirs into the path before execing the test binary; if
        // the directory does not exist, `chdir(2)` returns ENOENT and the
        // spawn fails with `FileNotFound` — even though the binary itself
        // was compiled successfully and exists in the zig cache.
        content.push_str(&format!(
            "    const {test_name}_run = b.addRunArtifact({test_name}_tests);\n"
        ));
        if has_file_fixtures {
            content.push_str(&format!(
                "    {test_name}_run.setCwd(b.path(\"{test_documents_path}\"));\n"
            ));
        }
        // Inject configured environment variables in alphabetical order.
        let mut sorted_env: Vec<_> = env.iter().collect();
        sorted_env.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in sorted_env {
            content.push_str(&format!(
                "    {test_name}_run.setEnvironmentVariable(\"{key}\", \"{value}\");\n"
            ));
        }
        if needs_mock_server {
            // Forward the captured mock-server URL into the test binary's
            // environment so `std.c.getenv(\"MOCK_SERVER_URL\")` resolves to the
            // live ephemeral address.
            content.push_str("    if (mock_server_url) |_url| {\n");
            content.push_str(&format!(
                "        {test_name}_run.setEnvironmentVariable(\"MOCK_SERVER_URL\", _url);\n"
            ));
            content.push_str("    }\n");
            content.push_str("    if (mock_servers_json) |_json| {\n");
            content.push_str(&format!(
                "        {test_name}_run.setEnvironmentVariable(\"MOCK_SERVERS\", _json);\n"
            ));
            content.push_str("    }\n");
            content.push_str("    {\n");
            content.push_str("        var _it = mock_servers_map.iterator();\n");
            content.push_str("        while (_it.next()) |_entry| {\n");
            content.push_str(&format!(
                "            {test_name}_run.setEnvironmentVariable(_entry.key_ptr.*, _entry.value_ptr.*);\n"
            ));
            content.push_str("        }\n");
            content.push_str("    }\n");
        }

        // Sequence test runs to prevent cache races. All tests (including download_test)
        // depend on the previous test, ensuring serial execution rather than parallel.
        // This prevents download_test's clean_cache() from racing with other tests'
        // cache lookups.
        if let Some(prev_name) = &prev_run {
            // Depend on the previous test to enforce serial execution
            content.push_str(&format!("    {test_name}_run.step.dependOn(&{prev_name}.step);\n"));
        }
        content.push_str(&format!("    test_step.dependOn(&{test_name}_run.step);\n\n"));
        prev_run = Some(format!("{test_name}_run"));
    }

    content.push_str("}\n");
    content
}

/// Emit the `build.zig` block that spawns the standalone mock-server binary at
/// configure time and captures its URL.
///
/// The mock-server binds an ephemeral `127.0.0.1` port and prints
/// `MOCK_SERVER_URL=http://127.0.0.1:<port>` (plus an optional
/// `MOCK_SERVERS={...}` JSON line for host-root fixtures) on stdout once it is
/// listening. The block produces three bindings consumed by the test run steps:
///   * `mock_server_url: ?[]const u8` — the base URL, or `null` when no binary
///     was found and no preset env var was supplied.
///   * `mock_servers_json: ?[]const u8` — the raw `MOCK_SERVERS=` JSON payload.
///   * `mock_servers_map: std.StringHashMap([]const u8)` — `MOCK_SERVER_<ID>`
///     env-var name → per-fixture URL, for host-root fixtures.
///
/// The spawned child is intentionally not awaited: it lives for the duration of
/// the `zig build` process, which spans test execution. A pre-set
/// `MOCK_SERVER_URL` short-circuits the spawn. Targets Zig 0.16 std APIs.
fn render_zig_mock_server_spawn() -> &'static str {
    r#"    const _alloc = b.allocator;
    var mock_server_url: ?[]const u8 = b.graph.environ_map.get("MOCK_SERVER_URL");
    var mock_servers_json: ?[]const u8 = null;
    var mock_servers_map = std.StringHashMap([]const u8).init(_alloc);
    if (mock_server_url == null) {
        const _bin = b.pathFromRoot("../rust/target/release/mock-server");
        const _fixtures = b.pathFromRoot("../../fixtures");
        var _threaded = std.Io.Threaded.init(_alloc, .{});
        const _io = _threaded.io();
        const _spawned = std.process.spawn(_io, .{
            .argv = &.{ _bin, _fixtures },
            .stdin = .pipe,
            .stdout = .pipe,
            .stderr = .inherit,
        });
        if (_spawned) |_child| {
            // The child is intentionally not awaited: it lives for the duration
            // of the `zig build` process, which spans test execution.
            const _stdout = _child.stdout.?;
            var _buf: [65536]u8 = undefined;
            var _file_reader = _stdout.readerStreaming(_io, &_buf);
            const _r = &_file_reader.interface;
            // Read startup lines: MOCK_SERVER_URL= then MOCK_SERVERS= (always
            // emitted, possibly `{}`). Cap the loop so a misbehaving server
            // cannot block the build indefinitely.
            var _saw_url = false;
            var _i: usize = 0;
            while (_i < 64) : (_i += 1) {
                const _line_raw = _r.takeDelimiterExclusive('\n') catch break;
                const _line = std.mem.trim(u8, _line_raw, " \r\t");
                if (std.mem.startsWith(u8, _line, "MOCK_SERVER_URL=")) {
                    mock_server_url = _alloc.dupe(u8, _line["MOCK_SERVER_URL=".len..]) catch null;
                    _saw_url = true;
                } else if (std.mem.startsWith(u8, _line, "MOCK_SERVERS=")) {
                    const _json = _line["MOCK_SERVERS=".len..];
                    mock_servers_json = _alloc.dupe(u8, _json) catch null;
                    if (std.json.parseFromSlice(std.json.Value, _alloc, _json, .{})) |_parsed| {
                        if (_parsed.value == .object) {
                            var _entries = _parsed.value.object.iterator();
                            while (_entries.next()) |_entry| {
                                if (_entry.value_ptr.* == .string) {
                                    const _key = std.fmt.allocPrint(_alloc, "MOCK_SERVER_{s}", .{_entry.key_ptr.*}) catch continue;
                                    for (_key) |*_c| _c.* = std.ascii.toUpper(_c.*);
                                    const _val = _alloc.dupe(u8, _entry.value_ptr.*.string) catch continue;
                                    mock_servers_map.put(_key, _val) catch {};
                                }
                            }
                        }
                    } else |_| {}
                    break;
                } else if (_saw_url) {
                    break;
                }
            }
        } else |_| {
            // Binary not built — leave mock_server_url null so tests surface a
            // clear connection error rather than a build failure.
        }
    }
"#
}

// ---------------------------------------------------------------------------
// HTTP server test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Renderer that emits Zig `test "..." { ... }` blocks targeting a mock server
/// via `std.http.Client`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
#[cfg(test)]
mod zig_build_tests {
    use super::{ZigBuildFlags, render_build_zig};
    use crate::e2e::config::DependencyMode;

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
        );

        // Must NOT reference the workspace-local target directory.
        assert!(
            !content.contains("../../target/release"),
            "registry mode build.zig must not reference workspace target dir, got:\n{content}"
        );

        // Must link the FFI from the dependency's bundled lib/ directory.
        assert!(
            content.contains("demo_client_dep.path(\"lib\")"),
            "registry mode build.zig must resolve FFI library path from fetched package's lib/ dir, got:\n{content}"
        );

        // Must link the C header from the dependency's bundled include/ directory.
        assert!(
            content.contains("demo_client_dep.path(\"include\")"),
            "registry mode build.zig must resolve FFI header path from fetched package's include/ dir, got:\n{content}"
        );

        // Must explicitly link the FFI system library.
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
        );

        // In local mode, workspace paths are expected for development.
        assert!(
            content.contains("../../target/release"),
            "local mode build.zig must reference workspace target dir for local development, got:\n{content}"
        );

        // Must link the FFI system library.
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
        );

        // All three vars must be present.
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

        // Alphabetical order: ALPHA < BETA < ZEBRA.
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
        );

        // With no env, no setEnvironmentVariable calls except the conditional mock-server ones.
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
        );

        // Verify no double-_run suffixes in dependOn calls.
        // This is the critical regression test: prev_run should not have _run appended again.
        // With the bug, identifiers like "conversion_run_run" would appear.
        assert!(
            !content.contains("_run_run"),
            "test step dependency must not contain '_run_run' (double suffix bug), but found in:\n{}",
            content
        );
    }
}
