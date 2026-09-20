use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use crate::core::template_versions::toolchain;
use crate::scaffold::{readme_language_configured, scaffold_meta};
use std::path::PathBuf;

pub(crate) fn scaffold_zig(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let ffi_lib_name = config.ffi_lib_name();
    let module_name = config.zig_module_name();
    let ffi_crate_path = config.ffi_crate_path();

    // Split per capsule type: the `b.dependency(...)` declaration and the library `module`
    // import must always run (the library itself may use the capsule regardless of whether a
    // test target exists), while `test_module.addImport` may only be emitted when a
    // `test_module` block actually exists below — see `test_seed`. ~keep
    let (module_capsule_imports, test_capsule_imports): (String, String) = config
        .zig
        .as_ref()
        .map(|c| {
            let import_names = crate::core::config::languages::zig_capsule_import_names(&c.capsule_types);
            let mut module_block = String::new();
            let mut test_block = String::new();
            for name in &import_names {
                module_block.push_str(&format!(
                    "\n    const {name}_dep = b.dependency(\"{name}\", .{{\n        \
                     .target = target,\n        .optimize = optimize,\n    }});\n    \
                     module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n"
                ));
                test_block.push_str(&format!(
                    "    test_module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n"
                ));
            }
            (module_block, test_block)
        })
        .unwrap_or_default();

    // A `zig build test` step is only emitted when `scaffold_zig_test` found something real to
    // assert against — the presence of the seed *is* the condition, so the step and the file it
    // points at can never disagree. A test target that compiles and runs against nothing — the
    // pre-fix defect, and even the seed's own "module imports successfully" fallback — passes on
    // an empty surface exactly the same as it would on a broken one: `zig build test` exits 0
    // either way. Omitting the step instead makes that state unmistakable: `zig build test`
    // fails with `error: no step named 'test'` rather than reporting false coverage. ~keep
    let test_seed = scaffold_zig_test(api, config, &module_name);
    let test_target_block = if test_seed.is_some() {
        format!(
            r#"
    // Scaffold also seeds `test/{module_name}_test.zig` (create-only — never overwrites
    // a real test suite once one exists) so `zig build test` has a real target to compile
    // from day one instead of silently re-running `src/{module_name}.zig` with zero `test`
    // blocks. ~keep
    const test_module = b.createModule(.{{
        .root_source_file = b.path("test/{module_name}_test.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    test_module.addImport("{module_name}", module);
    test_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    test_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    test_module.linkSystemLibrary("{ffi_lib}", .{{}});
{test_capsule_imports}
    const tests = b.addTest(.{{
        .root_module = test_module,
    }});

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
"#,
            module_name = module_name,
            ffi_lib = ffi_lib_name,
            test_capsule_imports = test_capsule_imports,
        )
    } else {
        String::new()
    };

    let build_zig = format!(
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Default library/include search paths follow the conventional Cargo workspace
    // layout. `alef publish package --lang zig` rewrites this file for the
    // distributed tarball so consumers link the bundled lib/ and include/ dirs.
    // Override with -Dffi_path=... and -Dffi_include_path=... if your layout differs.
    // Both are rebased onto this package's own build root before use: `.cwd_relative`
    // below resolves against the invoking process's working directory, so without this
    // the defaults only find anything when zig is run from inside this directory, and
    // never when the package is built as a `.path`/`.url` dependency of another
    // project -- which is exactly how alef's own snippet validator consumes it. ~keep
    const build_root = b.build_root.path orelse ".";

    const ffi_path_option = b.option(
        []const u8,
        "ffi_path",
        "Path to directory containing lib{ffi_lib}.{{dylib,so,dll,a}}"
    ) orelse "../../target/release";
    const ffi_path = b.pathResolve(&.{{ build_root, ffi_path_option }});

    const ffi_include_option = b.option(
        []const u8,
        "ffi_include_path",
        "Path to directory containing the FFI C header"
    ) orelse "{ffi_crate_path}/include";
    const ffi_include = b.pathResolve(&.{{ build_root, ffi_include_option }});

    const module = b.addModule("{module_name}", .{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    module.linkSystemLibrary("{ffi_lib}", .{{}});
{module_capsule_imports}{test_target_block}
    const example_module = b.createModule(.{{
        .root_source_file = b.path("examples/example.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    example_module.addImport("{module_name}", module);
    example_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    example_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    example_module.linkSystemLibrary("{ffi_lib}", .{{}});

    const example_exe = b.addExecutable(.{{
        .name = "example",
        .root_module = example_module,
    }});
    const run_example = b.addRunArtifact(example_exe);
    const example_step = b.step("example", "Run the example");
    example_step.dependOn(&run_example.step);
}}
"#,
        module_name = module_name,
        ffi_lib = ffi_lib_name,
        ffi_crate_path = ffi_crate_path,
        module_capsule_imports = module_capsule_imports,
        test_target_block = test_target_block,
    );

    let fingerprint = zig_fingerprint(&module_name);

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

    let example_zig = r#"const std = @import("std");

pub fn main() !void {
    var threaded: std.Io.Threaded = .init(std.heap.smp_allocator, .{});
    defer threaded.deinit();

    var stdout_buffer: [64]u8 = undefined;
    var stdout_writer = std.Io.File.stdout().writer(threaded.io(), &stdout_buffer);
    const stdout = &stdout_writer.interface;

    try stdout.print("Example: module loaded successfully\n", .{});
    try stdout.flush();
}
"#;

    let main_zig = format!(
        "// Generated by alef. Imports the full {module_name} API.\npub const api = @import(\"{module_name}.zig\");\n",
        module_name = module_name,
    );

    let mut files = vec![
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
    ];
    // Only seed `test/{module_name}_test.zig` — and only wire the matching `test_module`/
    // `test_step` into `build.zig` above — when `scaffold_zig_test` had a real, visible
    // function/type/enum to assert against. Both branches read the same `test_seed`, so the
    // step and its target file are emitted together or not at all. ~keep
    if let Some(test_seed) = test_seed {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("packages/zig/test/{module_name}_test.zig")),
            content: test_seed,
            generated_header: false,
        });
    }
    files.push(GeneratedFile {
        path: PathBuf::from("packages/zig/examples/example.zig"),
        content: example_zig.to_string(),
        generated_header: false,
    });
    files.push(GeneratedFile {
        path: PathBuf::from("packages/zig/src/main.zig"),
        content: main_zig.to_string(),
        generated_header: false,
    });
    // See the matching comment in `scaffold_swift`: once `[crates.readme.languages.zig]`
    // is configured, the README module owns this path end-to-end, and scaffold must not
    // compete with it as a second writer (#555). Inserted at its original position
    // (after `.editorconfig`, before the examples) rather than appended, so file order
    // is unchanged for languages that still rely on this placeholder. ~keep
    if !readme_language_configured(config, "zig") {
        files.insert(
            4,
            GeneratedFile {
                path: PathBuf::from("packages/zig/README.md"),
                content: readme,
                generated_header: false,
            },
        );
    }
    Ok(files)
}

/// Build the seed content for `test/{module_name}_test.zig`.
///
/// `build.zig`'s `test_module` now points here instead of re-compiling `src/{module_name}.zig`
/// (which carries zero `test` blocks — see the Defect-1 fix in `scaffold_zig`). `write_scaffold_files_report`
/// treats `generated_header: false` as create-only, so once a real suite exists at this path alef never
/// overwrites it; this only ever seeds a fresh project.
///
/// The seed must not be vacuous — a `zig build test` that always passes regardless of what alef
/// generated reproduces the exact "0 test blocks, silently green" defect one layer down. So this
/// asserts against the *real*, currently-generated API surface (`api`), in order of how strong a
/// check is safely synthesizable without knowing arbitrary function signatures:
///
/// 1. A visible zero-parameter function returning a bare primitive is actually called end-to-end
///    (real FFI link, real invocation) — the strongest check: it fails on a broken build, a link
///    error, or a removed/renamed export, not just a missing declaration.
/// 2. Otherwise, any other visible function is *referenced without being called*
///    (`_ = &{module}.{fn};`). Calling an arbitrary function generically isn't safe (unknown
///    allocator/ownership/JSON conversion needs per parameter), but a bare reference needs no
///    knowledge of the argument contract at all, and still forces Zig to semantically analyse
///    the wrapper's body and to resolve the extern C symbol that body calls. See
///    [`symbol_reference_test`] for the measured evidence and for the one class of change this
///    provably cannot catch.
/// 3. Otherwise, a visible type or enum is checked for existence via `@hasDecl` at comptime.
///    Types and enums have no referenceable-as-value form (`&SomeType` is not valid Zig), so
///    comptime existence is the strongest fact available about them — but this tier compiles no
///    wrapper body and links nothing, so it catches only a rename or a removal.
///
/// Every tier draws its candidate from `api` (the parsed Rust surface) and never from the
/// generated `.zig` file's declaration list. That is why the Zig binding generator's synthetic
/// helpers — `_last_error`, `_free_string`, `_error_with_message`, emitted directly as text by
/// `backends::zig::gen_bindings::helpers` with no backing `FunctionDef` — can never be picked as
/// the seed subject: they are structurally invisible here, not filtered out by name.
///
/// Returns `None` when no tier applies — a genuinely empty or fully-excluded surface (e.g.
/// scaffolding before any Rust code exists). There is nothing truthful to assert in that state,
/// so `scaffold_zig` seeds no `test/{module_name}_test.zig` and wires no `test_module`/
/// `test_step` into `build.zig` rather than emit a test that passes on nothing
/// indistinguishably from real coverage. A missing `test` step fails loudly (`error: no step
/// named 'test'`); a vacuous one reports false confidence. Returning `Option` rather than a
/// fallback string is what keeps the two decisions from drifting: the seed's own existence is
/// the condition `scaffold_zig` branches on, so there is no second predicate to fall out of
/// sync with these tiers. ~keep
fn scaffold_zig_test(api: &ApiSurface, config: &ResolvedCrateConfig, module_name: &str) -> Option<String> {
    let (exclude_functions, exclude_types) = zig_binding_exclusions(api, config);
    let function_is_visible = |f: &FunctionDef| !f.binding_excluded && !exclude_functions.contains(&f.name);
    let import_line = format!("const {module_name} = @import(\"{module_name}\");\n\n");

    let trivial_call_fn = api
        .functions
        .iter()
        .find(|f| function_is_visible(f) && f.params.is_empty() && matches!(f.return_type, TypeRef::Primitive(_)));
    if let Some(f) = trivial_call_fn {
        return Some(import_line + &trivial_call_test(module_name, f));
    }

    if let Some(f) = api.functions.iter().find(|f| function_is_visible(f)) {
        return Some(import_line + &symbol_reference_test(module_name, &f.name));
    }

    if let Some(t) = api
        .types
        .iter()
        .find(|t| !t.binding_excluded && !t.is_trait && !exclude_types.contains(&t.name))
    {
        return Some(import_line + &hasdecl_test(module_name, &t.name, "type"));
    }

    if let Some(e) = api
        .enums
        .iter()
        .find(|e| !e.binding_excluded && !exclude_types.contains(&e.name))
    {
        return Some(import_line + &hasdecl_test(module_name, &e.name, "enum"));
    }

    None
}

/// Names excluded from Zig binding generation, mirroring the same union `ZigBackend::generate_bindings`
/// (`src/backends/zig/gen_bindings/mod.rs`) computes: `[crates.zig]`/`[crates.ffi]` exclude lists plus
/// any type explicitly marked `binding_excluded`. Kept in sync deliberately rather than shared, since
/// the generator's version also folds in transitive signature exclusion this seed-picker doesn't need
/// to replicate (it only needs *a* safe, visible name — not the exhaustive filtered set).
fn zig_binding_exclusions(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut exclude_functions: std::collections::HashSet<String> = config
        .zig
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut exclude_types: std::collections::HashSet<String> = config
        .zig
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    (exclude_functions, exclude_types)
}

/// The strongest safe check: actually call a visible zero-arg, primitive-returning function
/// end-to-end (real FFI link, real invocation), so a broken build or a removed/renamed export
/// fails `zig build test` immediately instead of shipping green with a suite that links nothing.
fn trivial_call_test(module_name: &str, f: &FunctionDef) -> String {
    let call = if f.error_type.is_some() {
        format!("try {module_name}.{}()", f.name)
    } else {
        format!("{module_name}.{}()", f.name)
    };
    format!(
        "// Calls the generated `{fn_name}` binding end-to-end (real FFI link, real call), so a\n\
         // broken build or a removed/renamed export fails `zig build test` immediately instead of\n\
         // shipping green with a suite that links nothing. Create-only scaffold seed. ~keep\n\
         test \"{module_name}.{fn_name} runs\" {{\n    const result = {call};\n    _ = result;\n}}\n",
        fn_name = f.name,
    )
}

/// Reference — but do not call — a visible function, for the (common) case where no function in
/// the surface is safe to call generically.
///
/// `_ = &{module}.{fn};` needs no knowledge of the function's per-parameter ownership contract,
/// which is exactly why it is synthesizable where a real call is not. It is strictly stronger
/// than the comptime `@hasDecl` tier below it: taking the address of a function forces Zig to
/// semantically analyse that function's body and to resolve the extern C symbol the body calls,
/// so this tier fails on a link error or a wrapper-body type error, not merely on a rename.
///
/// Measured, not assumed — all three forms were run on Zig 0.16.0 with controls, in a
/// consumer tree. Against a wrapper whose extern symbol did not exist in the compiled library:
/// `@hasDecl` exited 0 ("All 1 tests passed"); `_ = &m.convert;` exited 1 with
/// `undefined symbol: _htm_probe_definitely_missing_symbol ... referenced by _lib.convert`; and
/// `_ = m.convert(1);` — the positive control — exited 1 with the same error. Against a type
/// error in the wrapper body with no extern involved: `@hasDecl` exited 0, `_ = &m.convert;`
/// exited 1 with `expected type '[]const u8', found 'c_int'`.
///
/// It cannot catch a C-level ABI change that preserves the symbol name; see the emitted comment,
/// which states that limit in the generated file where a reader of a green run will actually
/// encounter it. ~keep
fn symbol_reference_test(module_name: &str, name: &str) -> String {
    format!(
        "// `{name}` isn't a zero-arg, primitive-returning function this seed can safely call\n\
         // generically — its per-parameter allocator/ownership/JSON conversion contract is not\n\
         // knowable here — so this *references* it rather than calling it. That is not a weaker\n\
         // `@hasDecl`: taking the address forces Zig to semantically analyse the wrapper's body\n\
         // and to resolve the extern C symbol that body calls, neither of which a comptime\n\
         // `@hasDecl` does. Measured on Zig 0.16.0 with a deleted extern symbol: `@hasDecl` exits\n\
         // 0 (\"All 1 tests passed\"); this line exits 1 with `undefined symbol: ... referenced\n\
         // by ...`, matching a real call (the positive control). Same split for a type error in\n\
         // the wrapper body with no extern involved.\n\
         //\n\
         // LIMIT — read this before trusting a green run. This proves the symbol EXISTS and the\n\
         // wrapper typechecks. It does NOT prove the symbol is CORRECT. A C-level ABI change that\n\
         // preserves the symbol name is invisible to it: the linker resolves by name and C\n\
         // symbols carry no type information, so if the C signature changes and the generated Zig\n\
         // `extern` declaration is regenerated to match it, both move together and nothing ever\n\
         // disagrees. This closes \"the symbol does not exist\". It leaves \"the symbol lies\" wide\n\
         // open. Create-only scaffold seed. ~keep\n\
         test \"{module_name}.{name} symbol resolves\" {{\n    _ = &{module_name}.{name};\n}}\n",
    )
}

/// Comptime `@hasDecl` existence check against `name` (a real declaration in the currently
/// generated API surface). Used for types and enums, which — unlike functions — have no
/// referenceable-as-value form (`&SomeType` is not valid Zig), so comptime existence is the
/// strongest fact synthesizable about them.
///
/// This tier is deliberately the weakest one that still asserts something falsifiable. It
/// compiles no wrapper body and links nothing, so it catches a rename or a removal and nothing
/// else — a signature or ABI change that preserves the name passes it, and the symbol need not
/// exist in the compiled library at all for the test to go green. ~keep
fn hasdecl_test(module_name: &str, name: &str, kind: &str) -> String {
    format!(
        "// `{name}` isn't a zero-arg, primitive-returning function this seed can safely call\n\
         // generically, so this checks the generated {kind} exists at comptime instead. Create-only\n\
         // scaffold seed. ~keep\n\
         test \"{module_name} exposes `{name}`\" {{\n    \
             comptime {{\n        \
                 if (!@hasDecl({module_name}, \"{name}\")) {{\n            \
                     @compileError(\"{module_name} is missing expected declaration `{name}`\");\n        \
                 }}\n    \
             }}\n}}\n",
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{EnumDef, PrimitiveType, TypeDef};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
    }

    fn trivial_function(name: &str) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            ..Default::default()
        }
    }

    #[test]
    fn zig_seeded_test_sink_fires_with_a_contained_module_path() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-core"
sources = []
[crates.zig]
module_name = "safe_module"
"#,
        );
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..ApiSurface::default()
        };

        let files = scaffold_zig(&api, &config).expect("Zig scaffold renders");
        let seed = files
            .iter()
            .find(|file| file.path == std::path::Path::new("packages/zig/test/safe_module_test.zig"))
            .expect("the conditional seeded-test sink must fire");
        crate::core::config::output::validate_output_path(&seed.path).expect("seed path remains contained");
    }

    /// The strongest available check: a visible zero-arg, primitive-returning function is
    /// actually called, not just checked for existence.
    #[test]
    fn calls_a_visible_trivial_function_end_to_end() {
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("const my_lib = @import(\"my_lib\");"), "got:\n{out}");
        assert!(out.contains("test \"my_lib.ping runs\""), "got:\n{out}");
        assert!(out.contains("const result = my_lib.ping();"), "got:\n{out}");
        assert!(
            !out.contains("try my_lib.ping()"),
            "non-fallible call must not use try, got:\n{out}"
        );
    }

    /// A fallible zero-arg primitive function must be called with `try` — Zig's `test` blocks
    /// tolerate a returned error, so this compiles and surfaces a real runtime error as a
    /// genuine test failure instead of a Rust-side type mismatch.
    #[test]
    fn calls_a_fallible_trivial_function_with_try() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                error_type: Some("MyError".to_string()),
                ..trivial_function("ping")
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("const result = try my_lib.ping();"), "got:\n{out}");
    }

    /// A function that isn't zero-arg-and-primitive-returning can't be called generically
    /// (unknown allocator/ownership/JSON conversion needs), so it is *referenced* instead —
    /// which still forces Zig to analyse the wrapper body and resolve its extern C symbol,
    /// unlike the comptime `@hasDecl` tier below it. Pins the emitted line exactly.
    #[test]
    fn references_without_calling_a_function_that_fails_the_trivial_call_tier() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("test \"my_lib.greet symbol resolves\" {"), "got:\n{out}");
        assert!(out.contains("\n    _ = &my_lib.greet;\n"), "got:\n{out}");
        assert!(
            !out.contains("my_lib.greet("),
            "the reference tier must never synthesize a call, got:\n{out}"
        );
        assert!(
            // `symbol_reference_test`'s own `~keep` comment legitimately discusses `@hasDecl`
            // in prose (contrasting this stronger tier against it), so a bare substring check
            // false-positives on that comment. `if (!@hasDecl(` is `hasdecl_test`'s actual
            // functional emission (see line ~586) and is what "fell through" means. ~keep
            !out.contains("if (!@hasDecl("),
            "a visible function must not fall through to the weaker comptime tier, got:\n{out}"
        );
    }

    /// The emitted comment must state the one thing this tier provably cannot catch. An
    /// overclaiming comment is precisely how a green run gets misread as ABI verification,
    /// so the honest limit is part of the contract, not decoration.
    #[test]
    fn reference_tier_documents_that_it_cannot_catch_an_abi_change() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("It does NOT prove the symbol is CORRECT."), "got:\n{out}");
        assert!(
            out.contains("A C-level ABI change that"),
            "the limit must name the uncaught change class, got:\n{out}"
        );
        assert!(out.contains("~keep"), "got:\n{out}");
    }

    /// The T1 boundary is `matches!(return_type, TypeRef::Primitive(_))` — a *bare* primitive.
    /// An optional-returning function is the real-world shape that falls through it (an
    /// optional slice is what a fallible getter binding actually returns), and until now no
    /// test pinned that boundary with anything but `TypeRef::String`. `Optional(Primitive)` is
    /// the sharp case: it contains a primitive but is not one, so a `matches!` loosened to
    /// look inside the `Box` would silently start synthesizing an uncallable call.
    #[test]
    fn an_optional_return_falls_through_the_trivial_call_tier() {
        for return_type in [
            TypeRef::Optional(Box::new(TypeRef::String)),
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
        ] {
            let api = ApiSurface {
                functions: vec![FunctionDef {
                    name: "maybe_name".to_string(),
                    return_type: return_type.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

            assert!(
                out.contains("\n    _ = &my_lib.maybe_name;\n"),
                "{return_type:?} must fall through to the reference tier, got:\n{out}"
            );
            assert!(
                !out.contains("my_lib.maybe_name()"),
                "{return_type:?} is not a bare primitive and must never be called, got:\n{out}"
            );
        }
    }

    /// The Zig binding generator emits `_last_error`, `_free_string` and `_error_with_message` directly
    /// as text (`backends::zig::gen_bindings::helpers`) with no backing `FunctionDef`, so they
    /// exist as public declarations in the generated module but not in `ApiSurface`. Exclusion
    /// from the seed is therefore structural — every tier draws only from `api` — rather than a
    /// name filter that could be dropped. Pinned anyway: an implementation that ever picked its
    /// subject by inspecting the generated module's declarations would seed a test that asserts
    /// only against alef's own boilerplate and links nothing of the real surface, which is the
    /// vacuous-green failure this whole seed exists to prevent.
    #[test]
    fn never_seeds_against_the_synthetic_binding_helpers() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");
        for helper in ["_last_error", "_free_string", "_error_with_message"] {
            assert!(
                !out.contains(helper),
                "synthetic helper `{helper}` must never be the seed subject, got:\n{out}"
            );
        }
    }

    /// The `@hasDecl` tier's defining weakness, pinned so nobody mistakes it for a link check:
    /// it emits no call and no reference, so it compiles no wrapper body and asks the linker to
    /// resolve nothing. The subject name appears only as a comptime string argument — never as
    /// a field access on the module. Catches a rename or a removal, nothing more.
    #[test]
    fn hasdecl_tier_neither_calls_nor_references_its_subject() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Widget".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Widget\"))"), "got:\n{out}");
        assert!(out.contains("comptime {"), "got:\n{out}");
        assert!(
            !out.contains("my_lib.Widget"),
            "the comptime tier must not emit a field access, got:\n{out}"
        );
        assert!(
            !out.contains("_ = &"),
            "the comptime tier must not emit a symbol reference, got:\n{out}"
        );
        assert!(
            !out.contains("Widget()"),
            "the comptime tier must not emit a call, got:\n{out}"
        );
    }

    /// `binding_excluded` functions were never emitted into the generated `.zig` file, so the
    /// seed must skip them rather than asserting against a declaration that doesn't exist.
    #[test]
    fn skips_binding_excluded_functions() {
        let api = ApiSurface {
            functions: vec![
                FunctionDef {
                    binding_excluded: true,
                    ..trivial_function("hidden")
                },
                trivial_function("visible"),
            ],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("my_lib.visible"), "got:\n{out}");
        assert!(!out.contains("hidden"), "got:\n{out}");
    }

    /// `[crates.zig] exclude_functions` mirrors the real binding generator's own filter
    /// (`ZigBackend::generate_bindings`), so a function excluded there must also be skipped
    /// here — otherwise the seed asserts against a declaration the real generator never emits.
    #[test]
    fn skips_functions_excluded_via_zig_config() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []

[crates.zig]
exclude_functions = ["ping"]
"#,
        );
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        assert!(
            scaffold_zig_test(&api, &config, "my_lib").is_none(),
            "the only function is excluded, leaving nothing visible to assert against — seeding \
             anything here would assert against a declaration the real generator never emits"
        );
    }

    /// With no visible function at all, a visible type is checked for existence instead.
    #[test]
    fn falls_back_to_hasdecl_for_a_type_when_no_functions_exist() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Widget".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Widget\"))"), "got:\n{out}");
    }

    /// With no visible function or type, a visible enum is checked for existence instead.
    #[test]
    fn falls_back_to_hasdecl_for_an_enum_when_no_functions_or_types_exist() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib").expect("a visible item must produce a seed");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Color\"))"), "got:\n{out}");
    }

    /// A genuinely empty API surface (no Rust code written yet) has nothing to assert against,
    /// so the seed itself must be absent — that `None` is the single condition `scaffold_zig`
    /// branches on for both the seed file and `build.zig`'s `test` step.
    #[test]
    fn seeds_nothing_for_an_empty_api_surface() {
        assert!(scaffold_zig_test(&ApiSurface::default(), &minimal_config(), "my_lib").is_none());
    }

    /// Mirror of the empty-surface case for each visible kind: any one of a function, type, or
    /// enum is enough to produce a seed.
    #[test]
    fn seeds_when_any_single_kind_is_visible() {
        let with_function = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        let with_type = ApiSurface {
            types: vec![TypeDef {
                name: "Widget".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let with_enum = ApiSurface {
            enums: vec![EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for api in [with_function, with_type, with_enum] {
            assert!(scaffold_zig_test(&api, &minimal_config(), "my_lib").is_some());
        }
    }

    /// Defect-1 fix, direction 1: with no real function/type/enum to seed a genuine assertion
    /// against, `scaffold_zig` must not wire a `test` step into `build.zig` at all — a step that
    /// always passes on nothing is indistinguishable from real coverage, the exact defect this
    /// gate exists to close. Also asserts no `test/<module>_test.zig` file is produced, since a
    /// seeded file with nothing truthful to say would just recreate the same problem one layer
    /// down.
    #[test]
    fn build_zig_has_no_test_step_when_api_surface_is_empty() {
        let files = scaffold_zig(&ApiSurface::default(), &minimal_config()).expect("scaffold");

        let build_zig = &files
            .iter()
            .find(|f| f.path == std::path::Path::new("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content;
        assert!(!build_zig.contains("test_module"), "got:\n{build_zig}");
        assert!(!build_zig.contains("b.addTest"), "got:\n{build_zig}");
        assert!(!build_zig.contains("b.step(\"test\""), "got:\n{build_zig}");
        assert!(
            !files
                .iter()
                .any(|f| f.path == std::path::Path::new("packages/zig/test/my_lib_test.zig")),
            "no test file should be seeded when there is nothing to assert against, got: {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    /// Defect-1 fix, direction 2: with a real, visible function to seed against, `scaffold_zig`
    /// must still emit the `test` step, and it must point at the seeded
    /// `test/<module>_test.zig`, not back at `src/<module>.zig` (the zero-`test`-block
    /// generated bindings file the original defect ran tests against).
    #[test]
    fn build_zig_has_a_test_step_pointing_at_the_seed_when_api_surface_is_non_empty() {
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        let files = scaffold_zig(&api, &minimal_config()).expect("scaffold");

        let build_zig = &files
            .iter()
            .find(|f| f.path == std::path::Path::new("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content;
        assert!(
            build_zig.contains(".root_source_file = b.path(\"test/my_lib_test.zig\"),"),
            "got:\n{build_zig}"
        );
        assert!(
            build_zig.contains("b.step(\"test\", \"Run unit tests\");"),
            "got:\n{build_zig}"
        );
        assert!(
            !build_zig.contains(".root_source_file = b.path(\"src/my_lib.zig\"),\n        .target = target,\n        .optimize = optimize,\n        .link_libc = true,\n    });\n    test_module"),
            "test target must never point at the generated bindings module, got:\n{build_zig}"
        );

        let test_file = files
            .iter()
            .find(|f| f.path == std::path::Path::new("packages/zig/test/my_lib_test.zig"))
            .expect("test/my_lib_test.zig must be seeded when the api surface is non-empty");
        assert!(
            test_file.content.contains("test \"my_lib.ping runs\""),
            "got:\n{}",
            test_file.content
        );
    }

    fn build_zig_of(config: &ResolvedCrateConfig) -> String {
        scaffold_zig(&ApiSurface::default(), config)
            .expect("scaffold")
            .into_iter()
            .find(|f| f.path == std::path::Path::new("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content
    }

    /// Regression for a real defect found in html-to-markdown: the `ffi_include_path` default
    /// must come from `[crates.output] ffi`, never from a `{crate name}-ffi` template.
    ///
    /// This template *was* the derivation — `scaffold_zig` used a local
    /// `format!("{}-ffi", config.name)` until it was switched to `config.ffi_crate_path()`,
    /// which consults `[crates.output] ffi` first. The two agree only when the alef crate name
    /// happens to equal the FFI crate's directory stem, and in two of the three consumer repos
    /// it does not: html-to-markdown's crate is named `html-to-markdown-rs` while its FFI crate
    /// is `crates/html-to-markdown-ffi`, and tree-sitter-language-pack's crate is named
    /// `tree-sitter-language-pack` while its FFI crate is `crates/ts-pack-core-ffi`. Both repos
    /// carry a hand commit fixing the emitted default, and html-to-markdown additionally carries
    /// a `crates/html-to-markdown-rs-ffi/` directory holding nothing but a README — the tombstone
    /// the bad default pointed at. This pins the fix against the html-to-markdown shape verbatim,
    /// because the failure is silent: the path is only a default, so a build that overrides
    /// `-Dffi_include_path=` never notices it is wrong. ~keep
    #[test]
    fn ffi_include_default_follows_configured_output_path_not_the_crate_name() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "html-to-markdown-rs"
sources = []

[crates.output]
ffi = "crates/html-to-markdown-ffi/src/"
"#,
        );
        let build_zig = build_zig_of(&config);

        assert!(
            build_zig.contains(") orelse \"../../crates/html-to-markdown-ffi/include\";"),
            "got:\n{build_zig}"
        );
        assert!(
            !build_zig.contains("html-to-markdown-rs-ffi"),
            "the crate-name template must not leak back in, got:\n{build_zig}"
        );
    }

    /// With no `[crates.output] ffi` configured there is nothing better to go on, so the
    /// `crates/{name}-ffi` convention is the honest fallback — pinned so the fix above is
    /// understood as a precedence change, not as removing the convention.
    #[test]
    fn ffi_include_default_falls_back_to_the_crate_name_convention_when_unconfigured() {
        let build_zig = build_zig_of(&minimal_config());

        assert!(
            build_zig.contains(") orelse \"../../crates/my-lib-ffi/include\";"),
            "got:\n{build_zig}"
        );
    }

    /// Regression: both FFI search paths are attached with `.{ .cwd_relative = ... }`, which zig
    /// resolves against the *invoking process's* working directory. With the raw defaults the
    /// package therefore only builds when zig is run from inside `packages/zig` — `zig build
    /// --build-file packages/zig/build.zig` from the repo root fails with `unable to open library
    /// directory '../../target/release'`, and consuming the package as a `.path` dependency (which
    /// is exactly what the Zig snippet validator does) fails with `C import failed`. Rebasing both
    /// defaults onto `b.build_root` makes the paths independent of the caller's cwd. ~keep
    #[test]
    fn ffi_search_paths_resolve_against_the_packages_own_build_root() {
        let build_zig = build_zig_of(&minimal_config());

        assert!(
            build_zig.contains("const build_root = b.build_root.path orelse \".\";"),
            "got:\n{build_zig}"
        );
        assert!(
            build_zig.contains("const ffi_path = b.pathResolve(&.{ build_root, ffi_path_option });"),
            "got:\n{build_zig}"
        );
        assert!(
            build_zig.contains("const ffi_include = b.pathResolve(&.{ build_root, ffi_include_option });"),
            "got:\n{build_zig}"
        );
    }

    /// The values the `-D` overrides bind must stay directly readable as `orelse "<literal>"`:
    /// `snippets::validators::zig::binding_default` reads them straight out of the emitted text to
    /// reconstruct `-I` flags, and both migrations below locate the include default the same way. ~keep
    #[test]
    fn rebasing_keeps_both_option_defaults_readable_as_orelse_literals() {
        let build_zig = build_zig_of(&minimal_config());

        assert!(
            build_zig.contains("\"ffi_path\",\n        \"Path to directory containing libmy_lib_ffi.{dylib,so,dll,a}\"\n    ) orelse \"../../target/release\";"),
            "got:\n{build_zig}"
        );
        assert!(
            build_zig.contains("\"ffi_include_path\",\n        \"Path to directory containing the FFI C header\"\n    ) orelse \"../../crates/my-lib-ffi/include\";"),
            "got:\n{build_zig}"
        );
    }

    /// Read the module name out of a scaffolded `build.zig` the way the snippet validator does
    /// (`snippets::validators::zig`'s `zig_package_module` scans for the same `addModule("`
    /// marker), so this test fails for the same reason a real `zig build` of a snippet would.
    fn declared_module_name(build_zig: &str) -> &str {
        let marker = "addModule(\"";
        let start = build_zig.find(marker).expect("build.zig declares a module") + marker.len();
        let end = start + build_zig[start..].find('"').expect("module name is terminated");
        &build_zig[start..end]
    }

    // A documentation snippet's `@import` and the module `build.zig` declares are two producers
    // of one name. When they disagree the snippet does not merely look wrong -- `zig build`
    // fails outright with `no module named '<x>' available within module 'root'`, which is how
    // every generated Zig snippet in a consumer repo failed at once.
    //
    // This test lives in the scaffold module because it is the only place that can see both
    // producers: `scaffold::languages` is private to `scaffold`, while the e2e codegen is `pub`.
    // Both names are read from generated output rather than written as literals, so the test
    // cannot pin a matching pair of mistakes. `[crates.zig] module_name` is set here precisely
    // because it is the discriminating case: without it both sides fall back to the crate name
    // and would agree by accident. ~keep
    #[test]
    fn scaffolded_build_zig_declares_the_module_the_snippet_imports() {
        use crate::e2e::codegen::E2eCodegen as _;

        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []

[crates.zig]
module_name = "my_lib_rs"
"#,
        );
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };

        let files = scaffold_zig(&api, &config).expect("scaffold");
        let build_zig = &files
            .iter()
            .find(|file| file.path == std::path::Path::new("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content;
        let module = declared_module_name(build_zig);

        let mut e2e = crate::e2e::config::E2eConfig::default();
        e2e.call.function = "ping".into();
        let fixture = crate::e2e::fixture::Fixture {
            id: "ping".into(),
            description: "Ping".into(),
            ..Default::default()
        };
        let snippet = crate::e2e::codegen::zig::ZigE2eCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("snippet renders");

        assert!(
            snippet.contains(&format!("@import(\"{module}\")")),
            "snippet must import the module build.zig declares (`{module}`):\n{snippet}"
        );
    }
}
