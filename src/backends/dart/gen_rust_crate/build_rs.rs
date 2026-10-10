//! Emission of the generated dart bridge crate's `build.rs` and `flutter_rust_bridge.yaml`.

use crate::backends::dart::template_env;
use crate::core::backend::GeneratedFile;
use std::path::PathBuf;

pub(crate) fn emit_build_rs(rust_dir: &str, package_name: &str, module_name: &str, stem: &str) -> GeneratedFile {
    let loader_patch = render_loader_patch_fn(package_name, module_name, stem);
    let cfg_gates_fn = render_cfg_gates_fn();
    let content = template_env::render(
        "rust_build_rs.rs.jinja",
        crate::alef_context! {
            loader_patch => loader_patch.as_str(),
            cfg_gates_fn => cfg_gates_fn.as_str(),
        },
    );
    // Same ownership rail as the manifest above: `.rs` is markable, so an unmarked build.rs is
    // frozen out of every later regen. ~keep
    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/build.rs")),
        content,
        generated_header: true,
    }
}

/// Render the `patch_published_loader` Rust function embedded in the generated
/// dart bridge crate's `build.rs`.
///
/// flutter_rust_bridge's default loader uses a build-tree-relative `ioDirectory`
/// (e.g. `rust/target/release/`) resolved against the *consumer's* current
/// working directory — a path that is not shipped in the published pub tarball.
/// Consuming the package from pub.dev therefore fails to find the library and
/// falls back to opening a relative framework path (rejected by hardened
/// runtimes). This patcher injects a loader that resolves the prebuilt library
/// from the package's own installed location (`lib/src/<module>_bridge_generated/`,
/// resolved via `Isolate.resolvePackageUri`) as an absolute path, falling back
/// to flutter_rust_bridge's default loader when that library is absent (e.g.
/// local development builds). The patch is idempotent (keyed off a marker) and a
/// no-op when the FRB entrypoint signature is absent.
fn render_loader_patch_fn(package_name: &str, module_name: &str, stem: &str) -> String {
    let dart_replacement = dart_init_prologue_replacement(package_name, module_name, stem);
    template_env::render(
        "rust_loader_patch_fn.rs.jinja",
        crate::alef_context! {
            module_name => module_name,
            dart_replacement => dart_replacement.as_str(),
        },
    )
}

/// Build the patched `RustLib.init` prologue Dart source: the loader helper
/// method followed by the original `init` signature with a resolution line that
/// prefers the package-relative library.
///
/// Kept in sync with the FRB 2.x `RustLib.init` signature. Published pub.dev
/// packages stage natives under `lib/src/native/<rid>/` (e.g. `macos-arm64`,
/// `linux-x64`). For local FRB-dev builds the dylib is emitted into
/// `lib/src/{module}_bridge_generated/` and is searched as a fallback.
fn dart_init_prologue_replacement(package_name: &str, module_name: &str, stem: &str) -> String {
    template_env::render(
        "dart_init_prologue_replacement.jinja",
        crate::alef_context! {
            package_name => package_name,
            module_name => module_name,
            stem => stem,
        },
    )
}

/// Render the `carry_frb_cfg_gates` helper embedded in the generated dart bridge
/// crate's `build.rs`.
///
/// flutter_rust_bridge is not feature-aware: it bakes a wire wrapper and dispatch
/// arm for every `pub fn` it sees, gated or not. `alef generate` injects the
/// `#[cfg(...)]` gates from `lib.rs` into the committed `frb_generated.rs` once
/// (see `PostBuildStep::CarryFrbCfgGates` / `carry_lib_rs_cfg_gates_into_frb_generated`
/// in `frb_rewrite::cfg_gates`), but that file is regenerated from scratch whenever
/// `flutter_rust_bridge_codegen` runs again, which drops the injected gates. This
/// build script only invokes FRB under the opt-in `ALEF_FRB_REGENERATE_ON_BUILD`
/// gate (alef #140), so this embedded copy only needs to re-apply the gates after
/// that opt-in run, mirroring the alef-side logic exactly since the generated
/// build.rs is a standalone crate with no dependency on alef itself.
fn render_cfg_gates_fn() -> String {
    template_env::render("rust_frb_cfg_gates_fn.rs.jinja", crate::alef_context! {})
}

pub(crate) fn emit_frb_yaml(rust_dir: &str, module_name: &str) -> GeneratedFile {
    // correct position (after crate-level #![allow] attrs) to avoid E0753.
    let content = template_env::render(
        "flutter_rust_bridge_yaml.jinja",
        crate::alef_context! {
            module_name => module_name,
        },
    );
    // Same ownership rail as the manifest above: `.yaml` is markable, so an unmarked config is
    // frozen out of every later regen. ~keep
    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/flutter_rust_bridge.yaml")),
        content,
        generated_header: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitted_build_rs_is_valid_rust() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        syn::parse_file(&file.content).expect("generated build.rs must be valid Rust");
    }

    /// alef #140: `build.rs` used to invoke `flutter_rust_bridge_codegen` unconditionally on
    /// every `cargo build`/`cargo test`/`cargo clippy`, racing alef's own post-build
    /// `RunCommand` invocation and regenerating with a different (incomplete) subset of
    /// alef's post-processing -- a real invocation the consumer never asked for, applied by
    /// a tool the consumer never ran. alef must own frb regeneration exclusively; `build.rs`
    /// may only regenerate when a developer explicitly opts in, since generated sources are
    /// already committed and correct as of the last `alef generate`.
    #[test]
    fn emitted_build_rs_does_not_regenerate_frb_by_default() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("ALEF_FRB_REGENERATE_ON_BUILD"),
            "build.rs must gate FRB regeneration behind an explicit opt-in env var; got:\n{}",
            file.content
        );
        let frb_invocation = file
            .content
            .find(r#"Command::new("flutter_rust_bridge_codegen")"#)
            .expect("build.rs must still be able to invoke flutter_rust_bridge_codegen for the opt-in path");
        let gate_check = file
            .content
            .find("ALEF_FRB_REGENERATE_ON_BUILD")
            .expect("gate check must exist");
        assert!(
            gate_check < frb_invocation,
            "the opt-in env var must be checked before flutter_rust_bridge_codegen is invoked; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains(r#""--no-deps-check""#),
            "the opt-in path must tolerate valid prerelease Dart dependencies"
        );
        syn::parse_file(&file.content).expect("generated build.rs must be valid Rust");
    }

    /// A default Cargo build must be source-tree read-only. In particular, carrying cfg gates
    /// mutates the committed FRB Rust output and therefore belongs behind the same explicit
    /// regeneration opt-in as every other post-codegen rewrite.
    #[test]
    fn emitted_build_rs_does_not_mutate_sources_before_opt_in_gate() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        let opt_in_gate = file
            .content
            .find("if !frb_regeneration_opted_in()")
            .expect("build.rs must return early unless regeneration is explicitly enabled");

        for mutation in [
            "carry_frb_cfg_gates();",
            "patch_published_loader();",
            "fix_handler_executor_calls();",
        ] {
            let mutation_call = file
                .content
                .find(mutation)
                .unwrap_or_else(|| panic!("build.rs must retain the opt-in mutation `{mutation}`"));
            assert!(
                opt_in_gate < mutation_call,
                "source mutation `{mutation}` must occur only after the regeneration opt-in gate; got:\n{}",
                file.content
            );
        }
    }

    #[test]
    fn emitted_build_rs_patches_published_loader_after_codegen() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("patch_published_loader();"),
            "build.rs must invoke the loader patch after codegen"
        );
        assert!(
            file.content.contains("fn patch_published_loader()"),
            "build.rs must define the loader patch"
        );
        assert!(
            file.content
                .contains(r#"../lib/src/sample_router_bridge_generated/frb_generated.dart"#),
            "build.rs must target the generated frb dart file"
        );
        assert!(
            file.content
                .contains("Isolate.resolvePackageUri(Uri.parse('package:sample_router/sample_router.dart'))"),
            "build.rs replacement must resolve the package URI"
        );
        assert!(
            file.content
                .contains("externalLibrary ??= await _alefResolveExternalLibrary();"),
            "build.rs replacement must prefer the package-relative library"
        );
    }

    /// The build.rs loader patch and the in-process `frb_init_prologue_replacement`
    /// (`frb_rewrite::external_library_loader`) both render `dart_init_prologue_replacement.jinja`
    /// — assert the embedded copy also reaches `nativeDownloadAndCacheLibrary()` on a cache
    /// miss, so the two call sites cannot silently diverge again.
    #[test]
    fn emitted_build_rs_downloads_and_caches_library_on_cache_miss() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("await nativeDownloadAndCacheLibrary()"),
            "build.rs replacement must call nativeDownloadAndCacheLibrary() on a cache miss, got:\n{}",
            file.content
        );
    }

    #[test]
    fn emitted_build_rs_runs_dart_format_after_patch() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("Command::new(\"dart\")")
                && file.content.contains("\"format\"")
                && file.content.contains("FRB_GENERATED_DART"),
            "build.rs must run `dart format` on the patched frb_generated.dart"
        );
    }

    #[test]
    fn emitted_build_rs_handles_loader_patch_write_error() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content
                .contains("if let Err(err) = std::fs::write(path, &patched)")
                && file
                    .content
                    .contains("cargo:warning=failed to write published-loader patch: {err}")
                && file.content.contains("return;"),
            "emitted build.rs must handle loader patch write errors"
        );
    }

    /// `packages/dart/**` is fully alef-owned: `carry_frb_cfg_gates()` must ship in the
    /// generated build.rs itself, not be hand-restored after every regen (#434). Without
    /// this, a plain `flutter_rust_bridge_codegen generate` at consumer build time (e.g.
    /// during `dart pub get`) rewrites `frb_generated.rs` from scratch and silently drops
    /// every `#[cfg(...)]` gate alef injected during `alef generate`.
    #[test]
    fn emitted_build_rs_carries_frb_cfg_gates_after_codegen() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("carry_frb_cfg_gates();"),
            "build.rs must invoke carry_frb_cfg_gates() after FRB codegen"
        );
        assert!(
            file.content.contains("fn carry_frb_cfg_gates()"),
            "build.rs must define carry_frb_cfg_gates()"
        );
        assert!(
            file.content
                .contains("fn cfg_gated_free_functions(lib_rs: &str) -> Vec<(String, String)>"),
            "build.rs must define cfg_gated_free_functions() to scan lib.rs for gated pub fns"
        );
        assert!(
            file.content
                .contains(r#"const FRB_GENERATED_RUST: &str = "src/frb_generated.rs";"#),
            "build.rs must target the generated frb rust file"
        );
        syn::parse_file(&file.content).expect("generated build.rs must be valid Rust");
    }

    /// The manifest and `lib.rs` are derived from one `collect_cfg_features` call on one surface,
    /// so they cannot disagree in memory. They disagreed on disk: `generate::write` treats a
    /// missing provenance marker on a *markable* extension (`.toml`, `.rs`, `.yaml` — see
    /// `generate::write::marker_comment_style`) as proof of foreign authorship and refuses the
    /// write, permanently, while `lib.rs` stamps its own header and is rewritten every run. A
    /// newly cfg-gated item then reached the generated crate as `#[cfg(feature = "X")]` against a
    /// frozen `[features]` table — `unexpected_cfg` for every re-emission of that gate.
    ///
    /// Asserts the positive first: the fixture really does emit a gate and the matching feature
    /// key, so the ownership assertion below is not passing over an empty surface. ~keep
    #[test]
    fn every_markable_dart_rust_crate_file_carries_a_provenance_marker_on_disk() {
        use crate::cli::pipeline::generate::{ensure_generated_header, marker_comment_style};
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::hash::content_has_alef_marker;
        use crate::core::ir::{ApiSurface, FunctionDef};

        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "count_tokens".to_string(),
                rust_path: "sample_lib::text::count_tokens".to_string(),
                cfg: Some(r#"feature = "text-metrics""#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };

        let files = crate::backends::dart::gen_rust_crate::emit(&api, &config).expect("dart backend generates files");

        let lib_rs = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("src/lib.rs"))
            .expect("lib.rs is generated");
        assert!(
            lib_rs.content.contains("#[cfg(feature = \"text-metrics\")]"),
            "control: the fixture must emit a cfg-gated bridge fn into lib.rs; got:\n{}",
            lib_rs.content
        );
        let cargo_toml = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml is generated");
        let parsed: toml::Value = toml::from_str(&cargo_toml.content).expect("generated Cargo.toml must be valid TOML");
        assert!(
            parsed["features"]
                .as_table()
                .expect("[features] is a table")
                .contains_key("text-metrics"),
            "control: the manifest must declare the gate's feature in memory; got:\n{}",
            cargo_toml.content
        );

        let markable: Vec<&str> = files
            .iter()
            .filter(|f| marker_comment_style(&f.path).is_some())
            .filter_map(|f| f.path.to_str())
            .collect();
        for name in ["Cargo.toml", "build.rs", "flutter_rust_bridge.yaml", "src/lib.rs"] {
            assert!(
                markable.iter().any(|path| path.ends_with(name)),
                "control: {name} must be on the ownership predicate, else this test examines nothing; \
                 markable set was {markable:?}"
            );
        }

        for file in files.iter().filter(|f| marker_comment_style(&f.path).is_some()) {
            let on_disk = if file.generated_header {
                ensure_generated_header(&file.path, &file.content)
            } else {
                file.content.clone()
            };
            assert!(
                content_has_alef_marker(&on_disk),
                "{} is written on a markable extension with no alef provenance marker, so \
                 `generate::write::write_files_report` refuses to overwrite it forever and its \
                 content freezes while lib.rs keeps regenerating; got:\n{}",
                file.path.display(),
                on_disk.lines().take(5).collect::<Vec<_>>().join("\n")
            );
        }
    }
}
