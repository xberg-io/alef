use super::scope::poly_paths;
use super::*;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

pub(super) fn make_config(crate_name: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn required_formatters_always_include_rustfmt_and_poly() {
    let tools: Vec<&str> = required_formatters(&[Language::Python])
        .iter()
        .map(|f| f.tool)
        .collect();
    assert!(tools.contains(&"rustfmt"));
    assert!(tools.contains(&"poly"));
    assert!(!tools.contains(&"cargo-sort"), "python has no cargo-sort residual");
}

#[test]
fn required_formatters_add_cargo_sort_for_residual_languages() {
    for language in [
        Language::Wasm,
        Language::Ffi,
        Language::Ruby,
        Language::Elixir,
        Language::R,
    ] {
        let tools: Vec<&str> = required_formatters(&[language]).iter().map(|f| f.tool).collect();
        assert!(tools.contains(&"cargo-sort"), "{language} runs a cargo-sort residual");
    }
}

#[test]
fn required_formatters_add_mix_only_when_elixir_is_generated() {
    let tools: Vec<&str> = required_formatters(&[Language::Elixir])
        .iter()
        .map(|f| f.tool)
        .collect();
    assert!(tools.contains(&"mix"), "Elixir must require `mix` on PATH");

    for language in [
        Language::Python,
        Language::Wasm,
        Language::Ffi,
        Language::Ruby,
        Language::R,
    ] {
        let tools: Vec<&str> = required_formatters(&[language]).iter().map(|f| f.tool).collect();
        assert!(
            !tools.contains(&"mix"),
            "{language} must not require `mix` (only Elixir generation does)"
        );
    }
}

#[test]
fn required_formatters_add_zig_only_when_zig_is_generated() {
    let tools: Vec<&str> = required_formatters(&[Language::Zig])
        .iter()
        .map(|formatter| formatter.tool)
        .collect();
    assert!(tools.contains(&"zig"), "Zig generation must require `zig` on PATH");

    let python_tools: Vec<&str> = required_formatters(&[Language::Python])
        .iter()
        .map(|formatter| formatter.tool)
        .collect();
    assert!(
        !python_tools.contains(&"zig"),
        "non-Zig generation must not require the Zig toolchain"
    );
}

#[test]
fn required_formatters_add_dart_only_when_dart_is_generated() {
    let tools: Vec<&str> = required_formatters(&[Language::Dart])
        .iter()
        .map(|formatter| formatter.tool)
        .collect();
    assert!(tools.contains(&"dart"), "Dart generation must require `dart` on PATH");

    let python_tools: Vec<&str> = required_formatters(&[Language::Python])
        .iter()
        .map(|formatter| formatter.tool)
        .collect();
    assert!(
        !python_tools.contains(&"dart"),
        "non-Dart generation must not require the Dart toolchain"
    );
}

#[test]
fn dart_residual_formats_package_local_e2e_and_registry_test_app() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["dart"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
[crates.e2e]
output = "generated-e2e"
[crates.e2e.call]
function = "run"
[crates.e2e.registry]
output = "generated-test-apps"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let repo = tempfile::tempdir().expect("tempdir");
    for relative in ["packages/dart", "generated-e2e/dart", "generated-test-apps/dart"] {
        std::fs::create_dir_all(repo.path().join(relative)).expect("create Dart output");
    }
    let steps = language_residuals(&config, Language::Dart, repo.path());

    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].command, "dart");
    assert_eq!(steps[0].args, vec!["format", "."]);
    assert_eq!(steps[0].work_dir, repo.path().join("packages/dart"));
    assert_eq!(steps[1].work_dir, repo.path().join("generated-e2e/dart"));
    assert_eq!(steps[2].work_dir, repo.path().join("generated-test-apps/dart"));
}

#[test]
fn formatter_error_includes_stdout_and_stderr() {
    let err = run_formatter(
        "sh",
        &["-c", "printf 'stdout text'; printf 'stderr text' >&2; exit 7"],
        Path::new("."),
    )
    .expect_err("formatter should fail");
    let msg = err.to_string();
    assert!(msg.contains("stdout text"), "missing stdout in error: {msg}");
    assert!(msg.contains("stderr text"), "missing stderr in error: {msg}");
}

#[test]
fn wasm_residual_is_cargo_sort_n_on_the_crate_dir() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "wasm residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "crates/sample-model-wasm"],
        "cargo sort -n arg must be the wasm crate directory"
    );
    assert_eq!(
        steps[0].work_dir,
        Path::new("/repo"),
        "wasm cargo sort runs at repo root"
    );
}

#[test]
fn wasm_residual_uses_configured_output_path() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]
[crates.output]
wasm = "crates/sample-pack-core-wasm/src/"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "crates/sample-pack-core-wasm"],
        "cargo sort arg must match the crate dir derived from the configured output path"
    );
}

#[test]
fn ffi_residual_is_cargo_sort_n_workspace_wide() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ffi, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "FFI residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "-w"],
        "FFI cargo sort must be workspace-wide with -n flag"
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn ruby_residual_sorts_the_native_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ruby, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "Ruby residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert_eq!(steps[0].args[1], "-n");
    assert!(
        steps[0].args[2].starts_with("ext/") && steps[0].args[2].ends_with("/native"),
        "cargo sort arg must target ext/<gem>/native, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/ruby"));
}

/// The scaffold (`scaffold::languages::ruby::ruby_native_manifest_path` and
/// `scaffold_ruby_cargo`) always creates `ext/{core_crate_dir}_rb/native`, independent of any
/// configured `gem_name`. This residual step used to target `ext/{ruby_gem_name}/native`
/// instead, which agrees with the scaffold only when `gem_name` is left at its default (the
/// crate name) -- a configured `gem_name` silently pointed `cargo sort` at a directory that
/// was never created, and the step failed with "No file found at" against a real consumer.
/// ~keep
#[test]
fn ruby_residual_targets_the_scaffolds_directory_not_a_configured_gem_name() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "crawlberg"
sources = ["src/lib.rs"]
[crates.ruby]
gem_name = "crawlberg_gem"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);

    // Sanity: the configured override really does diverge from the crate name, which is the
    // precondition for this test to distinguish the two naming schemes at all.
    assert_eq!(config.ruby_gem_name(), "crawlberg_gem");

    let steps = language_residuals(&config, Language::Ruby, Path::new("/repo"));
    assert_eq!(
        steps[0].args[2], "ext/crawlberg_rb/native",
        "must target the scaffold's actual directory (core_crate_dir + \"_rb\"), not the \
         configured gem_name -- got: {:?}",
        steps[0].args
    );
}

#[test]
fn elixir_residual_is_cargo_sort_then_mix_deps_get_then_mix_format() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Elixir, Path::new("/repo"));
    assert_eq!(
        steps.len(),
        3,
        "Elixir residual must be cargo sort, then mix deps.get, then mix format"
    );

    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert_eq!(steps[0].args[1], "-n");
    assert!(
        steps[0].args[2].starts_with("native/") && steps[0].args[2].ends_with("_nif"),
        "cargo sort arg must target native/<app>_nif, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/elixir"));

    assert_eq!(steps[1].command, "mix");
    assert_eq!(steps[1].args, vec!["deps.get"]);
    assert_eq!(
        steps[1].work_dir,
        Path::new("/repo/packages/elixir"),
        "mix deps.get must run in the elixir package dir so it resolves mix.exs"
    );

    assert_eq!(steps[2].command, "mix");
    assert_eq!(steps[2].args, vec!["format"]);
    assert_eq!(
        steps[2].work_dir,
        Path::new("/repo/packages/elixir"),
        "mix format must run in the elixir package dir so it resolves mix.exs"
    );
}

#[test]
fn r_residual_sorts_the_extendr_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::R, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "R residual must be a single cargo sort step");
    assert_eq!(steps[0].args, vec!["sort", "-n", "packages/r/src/rust"]);
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn zig_residual_formats_source_and_build_file_in_package_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package = dir.path().join("packages/zig");
    std::fs::create_dir_all(package.join("src")).expect("zig source dir");
    std::fs::write(package.join("build.zig"), "pub fn build() void {}\n").expect("Zig build file");
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Zig, dir.path());
    assert_eq!(steps.len(), 1, "Zig residual must be a single native formatter step");
    assert_eq!(steps[0].command, "zig");
    assert_eq!(steps[0].args, vec!["fmt", "src", "build.zig"]);
    assert_eq!(steps[0].work_dir, package);
}

#[test]
fn zig_residual_deduplicates_matching_source_and_package_roots() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package = dir.path().join("packages/zig");
    std::fs::create_dir_all(package.join("src")).expect("zig source dir");
    std::fs::write(package.join("src/sample.zig"), "pub const Sample = struct {};\n").expect("Zig source");
    std::fs::write(package.join("build.zig"), "pub fn build() void {}\n").expect("Zig build file");
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
[crates.output]
zig = "packages/zig/src"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().expect("resolvable config").remove(0);

    let steps = language_residuals(&config, Language::Zig, dir.path());

    assert_eq!(
        steps.len(),
        1,
        "one physical Zig root must produce one formatter invocation"
    );
    assert_eq!(steps[0].work_dir, package);
    assert_eq!(steps[0].args, vec!["fmt", "src", "build.zig"]);
}

#[test]
fn zig_residual_formats_configured_source_and_canonical_package_roots() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source_root = dir.path().join("sdk/zig-package");
    std::fs::create_dir_all(source_root.join("src")).expect("configured Zig source dir");
    std::fs::write(source_root.join("src/sample.zig"), "pub const Sample = struct {};\n")
        .expect("configured Zig source");
    let package_root = dir.path().join("packages/zig");
    std::fs::create_dir_all(&package_root).expect("canonical Zig package dir");
    std::fs::write(package_root.join("build.zig"), "pub fn build() void {}\n").expect("canonical Zig build file");
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
[crates.output]
zig = "sdk/zig-package/src"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().expect("resolvable config").remove(0);

    let steps = language_residuals(&config, Language::Zig, dir.path());

    assert_eq!(
        steps.len(),
        2,
        "configured source and canonical package roots both need formatting"
    );
    assert_eq!(steps[0].work_dir, package_root);
    assert_eq!(steps[0].args, vec!["fmt", "build.zig"]);
    assert_eq!(steps[1].work_dir, source_root);
    assert_eq!(steps[1].args, vec!["fmt", "src"]);
}

#[test]
fn zig_generation_formatting_is_a_stable_native_formatter_fixed_point() {
    if !is_tool_available("zig") {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let package = base.join("packages/zig");
    std::fs::create_dir_all(package.join("src")).expect("zig source dir");
    let source = package.join("src/sample.zig");
    let build_file = package.join("build.zig");
    let disordered_source = "pub const Event=union(enum){ok:struct{value:u32,},err:struct{code:u32,},};\n";
    let disordered_build = "const std=@import(\"std\");pub fn build(b:*std.Build)void{_=b;}\n";
    std::fs::write(&source, disordered_source).expect("unformatted Zig source");
    std::fs::write(&build_file, disordered_build).expect("unformatted Zig build file");

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().expect("resolvable config").remove(0);
    let only: HashSet<Language> = [Language::Zig].into_iter().collect();

    let zig_only = |tool: &str| tool == "zig";
    format_generated_reporting_with(&config, base, Some(&only), false, &zig_only).expect("first Zig formatting pass");
    let first_source = std::fs::read_to_string(&source).expect("first formatted source");
    let first_build = std::fs::read_to_string(&build_file).expect("first formatted build file");
    assert!(
        first_source.contains("pub const Event = union(enum) {"),
        "the native formatter must rewrite the generated Zig source: {first_source}"
    );
    assert!(
        first_build.contains("const std = @import(\"std\");"),
        "the native formatter must rewrite the Zig build file: {first_build}"
    );

    std::fs::write(&source, disordered_source).expect("re-disordered Zig source");
    std::fs::write(&build_file, disordered_build).expect("re-disordered Zig build file");
    format_generated_reporting_with(&config, base, None, false, &zig_only).expect("second Zig formatting pass");
    assert_eq!(
        std::fs::read_to_string(&source).expect("second formatted source"),
        first_source,
        "full regeneration must restore the source formatter fixed point"
    );
    assert_eq!(
        std::fs::read_to_string(&build_file).expect("second formatted build file"),
        first_build,
        "full regeneration must restore the build formatter fixed point"
    );
    run_formatter("zig", &["fmt", "--check", "src", "build.zig"], &package)
        .expect("the persisted package must pass zig fmt --check");
}

#[test]
fn csharp_has_no_residual() {
    let config = make_config("sample-model");
    assert!(language_residuals(&config, Language::Csharp, Path::new("/repo")).is_empty());
}

#[test]
fn languages_without_residuals_have_none() {
    let config = make_config("sample-model");
    for lang in [
        Language::Python,
        Language::Node,
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::Csharp,
    ] {
        assert!(
            language_residuals(&config, lang, Path::new("/repo")).is_empty(),
            "{lang:?} must have no residual native pass (poly formats it)"
        );
    }
}

#[test]
fn poly_paths_full_regen_is_repo_root() {
    let config = make_config("sample-model");
    let paths = poly_paths(&config, Path::new("/repo"), None, &[Language::Python]);
    assert_eq!(
        paths,
        vec![PathBuf::from("/repo")],
        "full regen formats the whole repo once"
    );
}

#[test]
fn poly_paths_partial_regen_scopes_to_existing_package_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    let py_dir = base.join(config.package_dir(Language::Python));
    std::fs::create_dir_all(&py_dir).unwrap();

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert_eq!(
        paths,
        vec![py_dir],
        "partial regen scopes to the changed language's package dir"
    );
}

#[test]
fn poly_paths_partial_regen_drops_nonexistent_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert!(paths.is_empty(), "nonexistent package dirs are dropped");
}

#[test]
fn poly_pass_formats_generated_python_when_poly_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let py_path = base.join("packages/python/foo.py");
    std::fs::create_dir_all(py_path.parent().unwrap()).unwrap();
    std::fs::write(&py_path, "x=1").unwrap();

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);

    format_generated(&config, base, None);

    let formatted = std::fs::read_to_string(&py_path).unwrap();
    if is_tool_available("poly") {
        assert_eq!(
            formatted, "x = 1\n",
            "with poly installed, `poly fmt --fix` must reformat the generated Python file"
        );
    } else {
        assert_eq!(formatted, "x=1", "without poly the file must be left untouched");
    }
}

#[test]
fn partial_regen_respects_relative_poly_discovery_excludes() {
    if !is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(
        base.join("poly.toml"),
        "[discovery]\nexclude = [\"/packages/python/**\"]\n",
    )
    .unwrap();
    let py_path = base.join("packages/python/foo.py");
    std::fs::create_dir_all(py_path.parent().unwrap()).unwrap();
    std::fs::write(&py_path, "x=1").unwrap();

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let only: HashSet<Language> = [Language::Python].into_iter().collect();

    format_generated(&config, base, Some(&only));

    assert_eq!(
        std::fs::read_to_string(&py_path).unwrap(),
        "x=1",
        "poly's `[discovery] exclude` is matched relative to the root poly is given; a partial \
         regen must format from `base_dir` so a `/packages/python/**` exclude still applies. \
         Handing poly `packages/python` as its own root makes the exclude stop matching and \
         poly reformats a file the repo's config excludes it from."
    );
}

#[test]
fn install_poly_hooks_is_noop_outside_git_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_poly_hooks(dir.path());
}

#[test]
fn max_poly_fmt_passes_is_bounded() {
    assert_eq!(
        MAX_POLY_FMT_PASSES, 3,
        "the convergence loop must be capped at 3 passes per the full-regen contract"
    );
}
