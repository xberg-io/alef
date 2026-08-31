//! End-to-end regression for alef#368: a real `napi build` invocation, in a fixture with
//! *both* a workspace-root `package.json` and a differently-named crate-local one, must bake
//! the crate-local name into napi-rs's generated output -- never the root's.
//!
//! alef#376 later added `--no-js` (see `napi_js_ownership_tests`), which suppresses napi-rs's
//! own generated loader (`index.js`) entirely -- that was this test's original observable, so
//! it can no longer read the loader to prove `--package-json-path` did anything. Verified
//! empirically (real `napi build --no-js` runs, see task #378 notes) that
//! `--package-json-path` remains observable under `--no-js`: napi-rs still resolves
//! `napi.binaryName` from whichever `package.json` it reads and bakes that name into the
//! emitted native artifact's filename (`<binaryName>.<platform-triple>.node`). A run with
//! `--package-json-path` omitted entirely (the pre-#368 defect, cwd-relative default) against
//! this exact fixture produces an artifact named after the workspace-root manifest's
//! `napi.binaryName`, not the crate-local one -- confirming this is still the same defect
//! shape #368 fixed, just observed through a different artifact now that the loader is gone.
//!
//! A fixture with only one `package.json` cannot distinguish this fix from the defect it
//! fixes: napi-rs's default (`<cwd>/package.json`) and the crate-local file would name the
//! same package either way. Two differently-named manifests -- each with its own
//! `napi.binaryName` -- are the whole point. This test runs the exact command
//! [`build_command_for`] emits -- not a hand-rolled equivalent -- so a regression in either the
//! command string or napi-rs's own resolution behavior is caught.
//!
//! Requires `npx` (and network access, on first run, to fetch `@napi-rs/cli` and the `napi`/
//! `napi-derive` crates) and a working `cargo`. Skips rather than fails when `npx` is not on
//! `PATH`, matching this repo's convention for tests that depend on an external toolchain
//! (see e.g. `snippets::validators::typescript`).

use super::*;
use crate::core::backend::{BuildConfig, BuildDependency};
use std::io::Write as _;
use std::path::Path;

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("fixture path must have a parent")).expect("create fixture dir");
    let mut file = std::fs::File::create(path).expect("create fixture file");
    file.write_all(contents.as_bytes()).expect("write fixture file");
}

/// Whether `npx` runs, not merely resolves: a version-manager shim (e.g. nvm) spawns fine then
/// exits non-zero, so a PATH-only check would leave the skip below unreachable and fire the
/// assert everywhere Node is absent. ~keep
fn npx_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("npx")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[test]
fn napi_build_bakes_the_crate_local_package_name_not_the_workspace_roots() {
    if !npx_is_runnable() {
        return;
    }

    let project = tempfile::tempdir().expect("create tempdir");
    let root = project.path();

    // The workspace-root package.json a real consumer monorepo has -- deliberately given its
    // own `napi.binaryName`, distinct from the binding crate's own, so a build that
    // accidentally reads this one instead produces an unambiguous, differently-named artifact.
    // ~keep
    write_file(
        &root.join("package.json"),
        r#"{"name":"workspace-root-pkg","version":"0.0.0","private":true,"napi":{"binaryName":"workspace-root-pkg"}}"#,
    );

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
    .expect("parse fixture alef.toml");
    let config = alef_cfg.resolve().expect("resolve fixture config").remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };
    let command = build_command_for(Language::Node, &build_config, &config, false);

    let crate_dir = root.join("crates/mylib-node");
    write_file(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"mylib-node\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = \
         [\"cdylib\"]\n\n[dependencies]\nnapi = { version = \"3\", default-features = false, features = \
         [\"napi9\"] }\nnapi-derive = \"3\"\n\n[build-dependencies]\nnapi-build = \"2\"\n",
    );
    write_file(
        &crate_dir.join("build.rs"),
        "fn main() {\n    napi_build::setup();\n}\n",
    );
    write_file(
        &crate_dir.join("src/lib.rs"),
        "#[macro_use]\nextern crate napi_derive;\n\n#[napi]\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    );
    // The crate-local package.json a real Node binding crate ships -- named differently from
    // the workspace root above, with its own `napi.binaryName` that `napi build --platform`
    // bakes into the emitted native artifact's filename (`<binaryName>.<platform-triple>.node`).
    // `napi.targets` matches a real multi-platform binding crate's manifest; it does not gate
    // whether the current-host artifact gets built. ~keep
    write_file(
        &crate_dir.join("package.json"),
        r#"{"name":"crate-local-pkg","version":"0.1.0","napi":{"binaryName":"crate-local-pkg","targets":["x86_64-apple-darwin","aarch64-apple-darwin","x86_64-unknown-linux-gnu"]}}"#,
    );

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(root)
        .status()
        .expect("spawn napi build");
    assert!(status.success(), "napi build must succeed: {command}");

    // `--no-js` (alef#376) suppresses napi-rs's generated loader entirely, so the loader can no
    // longer serve as the observable here -- the emitted `.node` artifact's filename is the
    // remaining place `--package-json-path` shows up: napi-rs reads `napi.binaryName` from
    // whichever `package.json` it resolves and bakes that name into
    // `<binaryName>.<platform-triple>.node`. ~keep
    let node_artifact = std::fs::read_dir(&crate_dir)
        .expect("read crate dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|name| name.ends_with(".node"))
        .unwrap_or_else(|| {
            panic!(
                "napi build must still produce a native .node artifact in {}",
                crate_dir.display()
            )
        });

    assert!(
        node_artifact.starts_with("crate-local-pkg."),
        "the emitted native artifact must be named after the crate-local package.json's \
         napi.binaryName, not the workspace root's: got {node_artifact}"
    );
    assert!(
        !node_artifact.starts_with("workspace-root-pkg."),
        "the emitted native artifact must never bake in the workspace-root package.json's \
         napi.binaryName -- exactly the alef#368 defect: got {node_artifact}"
    );
}
