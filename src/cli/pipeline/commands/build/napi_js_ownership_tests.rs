//! Regression coverage for the `index.js` collision between alef's scaffolded node crate and
//! `napi build --platform`'s own default JS output filename.
//!
//! `scaffold::languages::node::scaffold_node` writes `crates/<crate>-node/index.js` as
//! `generated_header: false` -- user-owned after scaffold, and the actual platform-dispatch
//! loader `package.json`'s `"main"` resolves to. `napi build --platform`'s own `--js` output
//! defaults to that exact filename, written into the same `-o` directory, in napi-rs's own
//! process -- entirely outside the `generated_header` / `.alef-ownership.toml` boundary, which
//! only guards alef's own `write_files_report` / `write_scaffold_files_report` paths. Nothing
//! marks the file as protected from napi-rs's point of view, so a routine `alef build` (or the
//! scaffolded `npm run build`, or `alef publish`) silently clobbered a hand-edited `index.js`.
//!
//! Unlike the `--dts` collision fixed by redirecting to [`NAPI_AUTO_DTS_FILENAME`], napi-rs
//! exposes `--no-js` to disable its own JS output outright: alef's hand-derived platform-dispatch
//! file already does everything napi-rs's auto-generated one would, so nothing consumes the
//! napi-rs version and there is no redirect target worth keeping around.
//!
//! [`NAPI_AUTO_DTS_FILENAME`]: crate::core::template_versions::npm::NAPI_AUTO_DTS_FILENAME

use super::*;
use crate::core::backend::{BuildConfig, BuildDependency};
use crate::core::config::NewAlefConfig;
use crate::core::template_versions as tv;

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

fn napi_build_config() -> BuildConfig {
    BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    }
}

/// Resolve a single-crate node config whose `[crates.output] node` points at `output_dir`, so
/// the produced command's `--manifest-path`/`-o` arguments target a real, writable directory
/// rather than the empty string a config with no explicit output override would produce.
///
/// `output_dir` is set directly on the resolved config rather than through `[crates.output]`
/// TOML: path-safety validation now rejects any absolute `[crates.output]` value at `resolve()`
/// time (it would let a hostile config value write generated files outside the project root),
/// but these tests need a real absolute tempdir to run a real `napi build` against. Setting the
/// resolved fields directly reproduces exactly what `resolve_output_paths` would have written
/// for a (now-disallowed) absolute override. ~keep
fn resolved_node_config(output_dir: &std::path::Path) -> crate::core::config::ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#;
    let alef_cfg: NewAlefConfig = toml::from_str(toml).expect("fixture config must parse");
    let mut config = alef_cfg.resolve().expect("fixture config must resolve").remove(0);
    config.explicit_output.node = Some(output_dir.to_path_buf());
    config.output_paths.insert("node".to_string(), output_dir.to_path_buf());
    config
}

/// Cheap guard on the command string itself: proves the flag is present, but -- per the shape
/// of every prior alef defect that hid inside "the command looked right" -- does not by itself
/// prove the file survives. [`napi_build_never_clobbers_alefs_scaffolded_index_js`] is the test
/// that actually proves that.
#[test]
fn napi_build_command_disables_napi_rs_own_js_output() {
    let output_dir = tempfile::tempdir().expect("failed to create temp dir");
    let config = resolved_node_config(output_dir.path());
    let build_config = napi_build_config();

    let command = build_command_for(Language::Node, &build_config, &config, false);

    assert!(
        command.contains("--no-js"),
        "napi build must disable its own JS binding output so it cannot overwrite alef's \
         scaffolded index.js: {command}"
    );
    assert!(
        command.contains("--platform"),
        "--no-js only takes effect alongside --platform: {command}"
    );
}

const FIXTURE_CARGO_TOML: &str = r#"[package]
name = "napi-ownership-fixture"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
napi = { version = "{napi}", default-features = false, features = ["napi4"] }
napi-derive = "{napi_derive}"

[build-dependencies]
napi-build = "{napi_build}"
"#;

const FIXTURE_LIB_RS: &str = r#"#[macro_use]
extern crate napi_derive;

#[napi]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;

const FIXTURE_BUILD_RS: &str = r#"fn main() {
    napi_build::setup();
}
"#;

const FIXTURE_PACKAGE_JSON: &str = r#"{ "name": "napi-ownership-fixture", "version": "0.1.0" }
"#;

/// The exact content [`scaffold::languages::node::scaffold_node`] would have written as the
/// crate's `index.js` -- alef doesn't need the real dispatch-generator output here, only a
/// stand-in for "content a consumer might have hand-edited after scaffold", since ownership
/// survives regardless of what the bytes are.
const USER_OWNED_INDEX_JS: &str = "// USER-AUTHORED after alef scaffold -- napi build must never overwrite this.\nmodule.exports = { marker: \"user-owned\" };\n";

/// Real-fixture regression test: builds an actual napi crate through the exact command
/// [`build_command_for`]'s `"napi"` arm constructs, and asserts the scaffolded `index.js`
/// survives byte-for-byte. A command-string assertion alone cannot prove this -- it would still
/// pass if `--no-js` were silently ignored or misspelled -- so this drives the real `napi build`
/// subprocess against a real crate and inspects the file napi-rs actually wrote to.
///
/// Skips gracefully when `npx` is unavailable, matching
/// `snippets::validators::typescript`'s `which::which("tsc")` gate for its own real-toolchain
/// tests.
#[test]
fn napi_build_never_clobbers_alefs_scaffolded_index_js() {
    if !npx_is_runnable() {
        return;
    }

    let crate_dir = tempfile::tempdir().expect("failed to create temp dir for napi fixture");
    let crate_path = crate_dir.path();

    std::fs::write(
        crate_path.join("Cargo.toml"),
        FIXTURE_CARGO_TOML
            .replace("{napi}", tv::cargo::NAPI)
            .replace("{napi_derive}", tv::cargo::NAPI_DERIVE)
            .replace("{napi_build}", tv::cargo::NAPI_BUILD),
    )
    .expect("failed to write fixture Cargo.toml");
    std::fs::create_dir_all(crate_path.join("src")).expect("failed to create fixture src/");
    std::fs::write(crate_path.join("src/lib.rs"), FIXTURE_LIB_RS).expect("failed to write fixture lib.rs");
    std::fs::write(crate_path.join("build.rs"), FIXTURE_BUILD_RS).expect("failed to write fixture build.rs");
    std::fs::write(crate_path.join("package.json"), FIXTURE_PACKAGE_JSON)
        .expect("failed to write fixture package.json");
    std::fs::write(crate_path.join("index.js"), USER_OWNED_INDEX_JS).expect("failed to write fixture index.js");

    let config = resolved_node_config(crate_path);
    let build_config = napi_build_config();
    let command = build_command_for(Language::Node, &build_config, &config, false);

    // `napi build --platform` looks up `package.json` relative to the process cwd, not
    // relative to `--manifest-path`/`-o` -- a real `alef build` inherits its cwd from wherever
    // the consumer invoked it (typically the project root, which the produced command's
    // relative `crates/<crate>-node/...` paths are already written against). This fixture's
    // crate dir carries its own `package.json`, satisfying that lookup directly. ~keep
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(crate_path)
        .output()
        .expect("failed to spawn napi build");
    assert!(
        output.status.success(),
        "napi build failed (command: {command}):\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let index_js_after =
        std::fs::read_to_string(crate_path.join("index.js")).expect("index.js must still exist after napi build");
    assert_eq!(
        index_js_after, USER_OWNED_INDEX_JS,
        "napi build --platform must never overwrite alef's scaffolded index.js"
    );

    let native_binding_written = std::fs::read_dir(crate_path)
        .expect("failed to read fixture crate dir")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".node"));
    assert!(
        native_binding_written,
        "napi build must still produce a platform-tagged .node binary with --no-js set"
    );
}
