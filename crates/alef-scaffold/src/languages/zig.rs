use crate::scaffold_meta;
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::ir::ApiSurface;
use alef_core::template_versions::toolchain;
use std::path::PathBuf;

pub(crate) fn scaffold_zig(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let ffi_lib_name = config.ffi_lib_name();
    let module_name = config.zig_module_name();

    let ffi_crate_dir = format!("{}-ffi", config.crate_config.name);
    let build_zig = format!(
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Default library/include search paths follow the conventional Cargo workspace
    // layout (`<workspace>/target/{{profile}}` and the FFI crate's `include/` dir).
    // Override with `-Dffi_path=...` and `-Dffi_include_path=...` if your layout differs.
    const ffi_path = b.option([]const u8, "ffi_path", "Path to directory containing lib{ffi_lib}.{{dylib,so,dll,a}}") orelse "../../target/debug";
    const ffi_include = b.option([]const u8, "ffi_include_path", "Path to directory containing the FFI C header") orelse "../../crates/{ffi_crate_dir}/include";

    const module = b.addModule("{module_name}", .{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
    }});
    module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    module.linkSystemLibrary("{ffi_lib}", .{{}});

    const test_module = b.createModule(.{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
    }});
    test_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    test_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    test_module.linkSystemLibrary("{ffi_lib}", .{{}});

    const tests = b.addTest(.{{
        .root_module = test_module,
    }});

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
}}
"#,
        module_name = module_name,
        ffi_lib = ffi_lib_name,
        ffi_crate_dir = ffi_crate_dir,
    );

    // build.zig.zon — Zig 0.14+ requires `.name` to be an enum literal; Zig 0.16+ requires
    // a `.fingerprint` field. We derive a stable 64-bit value from the module name so that
    // regeneration is deterministic.
    let fingerprint = zig_fingerprint(&module_name);
    let build_zig_zon = format!(
        r#".{{
    .name = .{module_name},
    .version = "{version}",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = .{{}},
    .paths = .{{
        "build.zig",
        "build.zig.zon",
        "src",
    }},
}}
"#,
        module_name = module_name,
        version = version,
        fingerprint = fingerprint,
        min_zig = toolchain::MIN_ZIG_VERSION,
    );

    let gitignore = "zig-cache/\nzig-out/\n.zig-cache/\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.zig]\nindent_style = space\nindent_size = 4\n";

    let readme = format!(
        r#"# {module_name}

{description}

## Installation

Install Zig from [ziglang.org](https://ziglang.org/download/).

## Building

```sh
zig build
zig build test
```

## Usage

Add to your `build.zig.zon`:

```
.dependencies = .{{
    .{module_name} = .{{
        .path = "path/to/{module_name}",
    }},
}},
```

## License

{license}
"#,
        module_name = module_name,
        description = meta.description,
        license = meta.license,
    );

    let example_zig = "const std = @import(\"std\");\n\npub fn main() !void {\n    var gpa = std.heap.GeneralPurposeAllocator(.{}){};\n    defer _ = gpa.deinit();\n    const allocator = gpa.allocator();\n\n    const stdout = std.io.getStdOut().writer();\n    try stdout.print(\"Example: module loaded successfully\\n\", .{});\n}\n";

    let main_zig = "// Main module stub for the Zig bindings\n// Replace with your actual API calls after code generation\n\npub fn add(a: i32, b: i32) i32 {\n    return a + b;\n}\n\ntest \"example\" {\n    const a: i32 = 1;\n    const b: i32 = 1;\n    try @import(\"std\").testing.expectEqual(a + b, 2);\n}\n";

    let github_workflow = format!(
        r#"name: Zig

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Zig
        uses: mlugg/setup-zig@v1
        with:
          version: 0.13.0
      - name: Build Zig project
        run: zig build --directory packages/zig
      - name: Run tests
        run: zig build test --directory packages/zig
"#
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/zig/build.zig"),
            content: build_zig,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/build.zig.zon"),
            content: build_zig_zon,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/README.md"),
            content: readme,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/examples/example.zig"),
            content: example_zig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/src/main.zig"),
            content: main_zig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(".github/workflows/zig.yml"),
            content: github_workflow,
            generated_header: false,
        },
    ])
}

/// Derive a deterministic 64-bit fingerprint from the package name.
/// Zig 0.16+ requires a `.fingerprint` field in `build.zig.zon` with structure
/// `(crc32_ieee(name) << 32) | id`, where `id` is a 32-bit value not equal to
/// `0x00000000` or `0xffffffff`. We use FNV-1a over the package name as the
/// stable id so regeneration is deterministic.
fn zig_fingerprint(name: &str) -> u64 {
    let name_crc = crc32_ieee(name.as_bytes());
    let mut id: u32 = 0x811c_9dc5;
    for byte in name.as_bytes() {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    ((name_crc as u64) << 32) | (id as u64)
}

fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
