use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::template_versions::toolchain;
use crate::scaffold::scaffold_meta;
use std::path::PathBuf;

pub(crate) fn scaffold_zig(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let ffi_lib_name = config.ffi_lib_name();
    let module_name = config.zig_module_name();
    let ffi_crate_dir = format!("{}-ffi", config.name);

    // Host-native capsule passthrough: each capsule type's `host_type` implies a Zig module
    // import name (e.g. `?*const my_mod.Language` → `my_mod`). build.zig must wire each
    // dependency's module into both the public module and the test module. Empty when no
    // capsule types are configured with a non-empty `package`.
    let capsule_imports_block: String = config
        .zig
        .as_ref()
        .map(|c| {
            // Collect import names from capsule types that have a package.
            let import_names = crate::core::config::languages::zig_capsule_import_names(&c.capsule_types);
            if import_names.is_empty() {
                return String::new();
            }
            let mut block = String::new();
            for name in &import_names {
                block.push_str(&format!(
                    "\n    const {name}_dep = b.dependency(\"{name}\", .{{\n        \
                     .target = target,\n        .optimize = optimize,\n    }});\n    \
                     module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n    \
                     test_module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n"
                ));
            }
            block
        })
        .unwrap_or_default();

    // Generate build.zig with local workspace defaults. `alef publish package --lang zig`
    // rewrites it to use bundled lib/ and include/ paths for fetched consumers.
    let build_zig = format!(
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Default library/include search paths follow the conventional Cargo workspace
    // layout. `alef publish package --lang zig` rewrites this file for the
    // distributed tarball so consumers link the bundled lib/ and include/ dirs.
    // Override with -Dffi_path=... and -Dffi_include_path=... if your layout differs.
    const ffi_path = b.option(
        []const u8,
        "ffi_path",
        "Path to directory containing lib{ffi_lib}.{{dylib,so,dll,a}}"
    ) orelse "../../target/release";

    const ffi_include = b.option(
        []const u8,
        "ffi_include_path",
        "Path to directory containing the FFI C header"
    ) orelse "../../crates/{ffi_crate_dir}/include";

    const module = b.addModule("{module_name}", .{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    module.linkSystemLibrary("{ffi_lib}", .{{}});

    const test_module = b.createModule(.{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    test_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    test_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    test_module.linkSystemLibrary("{ffi_lib}", .{{}});
{capsule_imports_block}
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
        capsule_imports_block = capsule_imports_block,
    );

    // build.zig.zon — Zig 0.14+ requires `.name` to be an enum literal; Zig 0.16+ requires
    // a `.fingerprint` field. We derive a stable 64-bit value from the module name so that
    // regeneration is deterministic.
    let fingerprint = zig_fingerprint(&module_name);

    // Host-native capsule (Language) passthrough: inject each capsule dependency into
    // build.zig.zon. The dependency key is derived from `host_type` (same import name used
    // in build.zig). Each capsule entry's `package` is the dependency URL and
    // `package_version` the URL hash (`.hash = ...`).
    let zig_capsule_deps: String = config
        .zig
        .as_ref()
        .map(|c| {
            let mut entries: Vec<String> = c
                .capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .filter_map(|cap| {
                    let import_name = crate::core::config::languages::zig_capsule_import_name(&cap.host_type)?;
                    let hash_field = if cap.package_version.is_empty() {
                        String::new()
                    } else {
                        format!("\n            .hash = \"{}\",", cap.package_version)
                    };
                    Some(format!(
                        "        .{import_name} = .{{\n            .url = \"{}\",{}\n        }},",
                        cap.package, hash_field
                    ))
                })
                .collect();
            entries.sort();
            entries.dedup();
            entries.join("\n")
        })
        .unwrap_or_default();
    let dependencies_block = if zig_capsule_deps.is_empty() {
        ".{}".to_string()
    } else {
        format!(".{{\n{zig_capsule_deps}\n    }}")
    };

    let build_zig_zon = format!(
        r#".{{
    .name = .{module_name},
    .version = "{version}",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = {dependencies_block},
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
        dependencies_block = dependencies_block,
    );

    let gitignore = "zig-cache/\nzig-out/\n.zig-cache/\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.zig]\nindent_style = space\nindent_size = 4\n";
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();

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

```text
.dependencies = .{{
    .{module_name} = .{{
        .path = "path/to/{module_name}",
    }},
}},
```
"#,
        module_name = module_name,
        description = meta.description,
    ) + &license_section;

    let example_zig = "const std = @import(\"std\");\n\npub fn main() !void {\n    var gpa = std.heap.GeneralPurposeAllocator(.{}){};\n    defer _ = gpa.deinit();\n    const allocator = gpa.allocator();\n\n    const stdout = std.io.getStdOut().writer();\n    try stdout.print(\"Example: module loaded successfully\\n\", .{});\n}\n";

    let main_zig = format!(
        "// Generated by alef. Imports the full {module_name} API.\npub const api = @import(\"{module_name}.zig\");\n",
        module_name = module_name,
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
