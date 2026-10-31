//! Compile the wasm backend's own output and read the JavaScript glue `wasm-bindgen` emits for it.
//!
//! Every other wasm test in this repo asserts on generated *Rust source text*, and that is
//! structurally blind to the defect class this file exists for: a by-value exported-struct
//! argument compiles perfectly, and only `wasm-bindgen`'s JS shim reveals that it was lowered
//! through `__destroy_into_raw()` -- destroying the caller's handle. alef#470 shipped, and
//! alef#472/#473 were filed, against output that every string assertion in the suite called
//! correct. The existing wasm lane in `tests/generated_output_downstream_gate.rs` compiles the
//! emitted crate for the *host* under `cargo clippy -D warnings`, which would also have passed
//! green on the pre-#470 bug.
//!
//! So this fixture runs the real pipeline end to end: extract a fixture core crate, generate its
//! wasm bindings with [`WasmBackend`], build them for `wasm32-unknown-unknown`, run the real
//! `wasm-bindgen` CLI, read which scalar arguments the resulting JS destroys, and execute the
//! array/copy and detached-getter contracts under Node.
//!
//! The assertion is an exact set, not a "no occurrences" grep. `__destroy_into_raw()` is
//! legitimate in `free()` (`this.__destroy_into_raw()`) and is currently unavoidable for a
//! constructor parameter typed `Option<T>`, because `Option<&T>` has no `OptionFromWasmAbi`
//! impl. Listing the survivors by class, member and argument name makes the remaining gap a
//! line someone had to write, and makes any *new* consumed handle a failure.

use std::path::{Path, PathBuf};

use crate::backends::wasm::WasmBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::test_support::{spawn_from_stable_dir, toolchain};

/// The fixture crate the bindings are generated from.
///
/// Extracted with alef's own extractor rather than hand-built as an `ApiSurface`, so the IR under
/// test is the IR a consumer would get. Each type is here for one emitter path:
/// `RenderOptions` for the non-`Default` constructor and the class-typed setters, `Layer` for the
/// tagged-data-enum payload setters, and `DefaultedOptions` for the `#[derive(Default)]`
/// constructor whose parameters are all `Option<T>` and therefore cannot be borrowed. ~keep
const FIXTURE_CORE_LIB_RS: &str = r#"//! Fixture core crate for the wasm-bindgen JS glue oracle.

/// A palette.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    /// Human-readable label.
    pub label: String,
}

/// Render options carrying a required class-typed field. Deliberately not `Default`.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Primary palette.
    pub palette: Palette,
    /// Window title.
    pub title: String,
    /// Palette consulted when the primary one has no entry.
    pub fallback_palette: Option<Palette>,
}

/// Options that derive `Default`, so every generated constructor parameter is `Option<T>`.
#[derive(Clone, Debug, Default)]
pub struct DefaultedOptions {
    /// Primary palette.
    pub palette: Palette,
    /// Window title.
    pub title: String,
}

/// A class-valued array, whose ABI necessarily takes ownership of each element.
#[derive(Clone, Debug, Default)]
pub struct PaletteCollection {
    /// Palettes in display order.
    pub palettes: Vec<Palette>,
}

/// A tagged data enum whose payload carries a class-typed field.
#[derive(Clone, Debug)]
pub enum Layer {
    /// A solid fill.
    Solid {
        /// Palette used for the fill.
        palette: Palette,
        /// Fill opacity.
        opacity: f64,
    },
    /// Nothing drawn.
    Blank,
}
"#;

const CRATE_NAME: &str = "fixture-core";
const BINDING_CRATE_NAME: &str = "fixture-core-wasm";

/// One `<argument>.__destroy_into_raw()` call the JS glue makes, with the class and member it
/// sits in. `this.__destroy_into_raw()` inside `free()` is not one of these -- see
/// [`consumed_handles`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ConsumedHandle {
    class: String,
    member: String,
    argument: String,
}

impl std::fmt::Display for ConsumedHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{} consumes `{}`", self.class, self.member, self.argument)
    }
}

/// Every argument the emitted JS destroys, keyed by where it does it.
///
/// Deliberately a line scanner rather than a JS parse: the glue is generated, its shape is
/// stable, and a parser dependency would be a second thing to keep honest. The receiver is the
/// identifier immediately left of `.__destroy_into_raw()`, and `this` is excluded because
/// `free()` legitimately consumes the object it is called on.
fn consumed_handles(js: &str) -> Vec<ConsumedHandle> {
    let mut found = Vec::new();
    let mut class = String::new();
    let mut member = String::new();
    for line in js.lines() {
        if let Some(rest) = class_declaration(line) {
            class = rest;
            member.clear();
            continue;
        }
        if let Some(declared) = member_declaration(line) {
            member = declared;
        }
        for argument in destroy_receivers(line) {
            if argument == "this" {
                continue;
            }
            found.push(ConsumedHandle {
                class: class.clone(),
                member: member.clone(),
                argument,
            });
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `export class WasmPalette {` -> `WasmPalette`; anything else -> `None`. The `export` prefix is
/// what `--target web` emits and `--target nodejs` does not, so both spellings are accepted.
fn class_declaration(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("class ")
        .or_else(|| line.strip_prefix("export class "))?;
    Some(rest.split_whitespace().next()?.to_string())
}

/// `    set palette(value) {` -> `set palette`; anything else -> `None`.
fn member_declaration(line: &str) -> Option<String> {
    let body = line.strip_prefix("    ")?;
    if body.starts_with(' ') || !line.trim_end().ends_with('{') {
        return None;
    }
    let name = body.split_once('(')?.0.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn destroy_receivers(line: &str) -> Vec<String> {
    const CALL: &str = ".__destroy_into_raw()";
    let mut receivers = Vec::new();
    let bytes = line.as_bytes();
    let mut search = 0;
    while let Some(offset) = line[search..].find(CALL) {
        let end = search + offset;
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || matches!(bytes[start - 1], b'_' | b'$')) {
            start -= 1;
        }
        if start < end {
            receivers.push(line[start..end].to_string());
        }
        search = end + CALL.len();
    }
    receivers
}

fn resolved_wasm_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "{CRATE_NAME}"
sources = ["src/lib.rs"]

[crates.wasm]
"#
    ))
    .expect("fixture config must parse");
    config.resolve().expect("fixture config must resolve").remove(0)
}

/// The `wasm-bindgen` crate version the CLI on `PATH` can read.
///
/// The CLI aborts outright on a schema mismatch ("rust Wasm file schema version" vs "this binary
/// schema version"), and alef emits a caret `wasm-bindgen = "0.2"` that resolves to the newest
/// 0.2.x -- which is routinely ahead of an installed CLI. The fixture manifest therefore pins
/// `=<this version>` instead of reusing what alef emits. ~keep
fn cli_crate_version(cli: &Path) -> String {
    let output = spawn_from_stable_dir(cli.as_os_str())
        .arg("--version")
        .output()
        .expect("run wasm-bindgen --version");
    let banner = String::from_utf8_lossy(&output.stdout).trim().to_string();
    banner
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("cannot read a version out of `wasm-bindgen --version` output {banner:?}"))
        .to_string()
}

/// Write the two-crate fixture workspace and return the generated binding crate's manifest path.
fn materialize_workspace(root: &Path, wasm_bindgen_version: &str) -> PathBuf {
    let core = root.join("core");
    let bindings = root.join("bindings");
    std::fs::create_dir_all(core.join("src")).expect("create fixture core crate");
    std::fs::create_dir_all(bindings.join("src")).expect("create fixture binding crate");

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"core\", \"bindings\"]\n",
    )
    .expect("write fixture workspace manifest");

    let core_lib = core.join("src/lib.rs");
    std::fs::write(&core_lib, FIXTURE_CORE_LIB_RS).expect("write fixture core source");
    std::fs::write(
        core.join("Cargo.toml"),
        format!("[package]\nname = \"{CRATE_NAME}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .expect("write fixture core manifest");

    let api = crate::extract::extractor::extract(&[core_lib.as_path()], CRATE_NAME, "0.1.0", None)
        .expect("extract the fixture core crate");
    let generated = WasmBackend
        .generate_bindings(&api, &resolved_wasm_config())
        .expect("generate wasm bindings for the fixture");
    let lib_rs = generated
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .expect("the wasm backend must emit a lib.rs");
    std::fs::write(bindings.join("src/lib.rs"), &lib_rs.content).expect("write generated bindings");

    let manifest = bindings.join("Cargo.toml");
    std::fs::write(
        &manifest,
        format!(
            "[package]\nname = \"{BINDING_CRATE_NAME}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\ncrate-type = [\"cdylib\"]\n\n\
             [dependencies]\n{CRATE_NAME} = {{ path = \"../core\" }}\n\
             wasm-bindgen = \"={wasm_bindgen_version}\"\n"
        ),
    )
    .expect("write fixture binding manifest");
    manifest
}

/// Build the binding crate for `wasm32-unknown-unknown` and return the produced module.
fn build_wasm_module(root: &Path, manifest: &Path) -> PathBuf {
    let target_dir = root.join("target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = spawn_from_stable_dir(&cargo)
        .arg("build")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("run cargo build for wasm32-unknown-unknown");
    assert!(
        output.status.success(),
        "alef's generated wasm crate did not build for wasm32-unknown-unknown.\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    target_dir
        .join("wasm32-unknown-unknown/debug")
        .join(format!("{}.wasm", BINDING_CRATE_NAME.replace('-', "_")))
}

/// Run the real CLI over the built module and return the emitted JavaScript glue.
fn emit_js_glue(cli: &Path, root: &Path, module: &Path) -> String {
    let out_dir = root.join("pkg-web");
    let output = spawn_from_stable_dir(cli.as_os_str())
        .arg("--target")
        .arg("web")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg(module)
        .output()
        .expect("run the wasm-bindgen CLI");
    assert!(
        output.status.success(),
        "wasm-bindgen could not process alef's generated module.\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let js = out_dir.join(format!("{}.js", BINDING_CRATE_NAME.replace('-', "_")));
    std::fs::read_to_string(&js).unwrap_or_else(|error| panic!("read emitted glue {js:?}: {error}"))
}

/// Emit a Node package and execute the ownership/copy contract against the real JS glue.
fn run_node_ownership_oracle(cli: &Path, root: &Path, module: &Path) {
    let out_dir = root.join("pkg-node");
    let output = spawn_from_stable_dir(cli.as_os_str())
        .arg("--target")
        .arg("nodejs")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg(module)
        .output()
        .expect("run the wasm-bindgen CLI for Node");
    assert!(
        output.status.success(),
        "wasm-bindgen could not emit the Node package.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let node =
        crate::test_support::tool_path_with_stable_path("node").expect("the wasm-bindgen runtime oracle requires Node");
    let module = out_dir.join(format!("{}.js", BINDING_CRATE_NAME.replace('-', "_")));
    let script = root.join("ownership-oracle.cjs");
    std::fs::write(
        &script,
        r#"const w = require(process.argv[2]);
const original = new w.WasmPalette("original");
new w.WasmPaletteCollection([original.copyForTransfer()]);
if (original.label !== "original") throw new Error("constructor consumed the retained handle");
const collection = w.WasmPaletteCollection.default();
collection.palettes = [original.copyForTransfer()];
if (original.label !== "original") throw new Error("setter consumed the retained handle");

const options = w.WasmDefaultedOptions.default();
const nested = options.palette;
nested.label = "changed";
if (options.palette.label === "changed") throw new Error("getter did not return a detached copy");
options.palette = nested;
if (options.palette.label !== "changed") throw new Error("read/modify/reassign did not update the parent");
if (nested.label !== "changed") throw new Error("borrowed setter consumed the copy");
process.stdout.write("ownership oracle ok");
"#,
    )
    .expect("write Node ownership oracle");
    let output = spawn_from_stable_dir(node.as_os_str())
        .arg(&script)
        .arg(&module)
        .output()
        .expect("run Node ownership oracle");
    assert!(
        output.status.success(),
        "Node ownership oracle failed.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ownership oracle ok");
}

/// Locate the `wasm-bindgen` CLI, or report that this fixture did not run.
///
/// Routed through the crate-wide [`toolchain::WASM_BINDGEN`] gate, which counts the invocation as
/// attempted and then as executed or skipped; `scripts/toolchain-census.sh --require wasm-bindgen`
/// turns "0 of 1 executed" into a failed CI leg. A bare `eprintln!` skip notice would be
/// invisible here, because `libtest` captures the stderr of a *passing* test. ~keep
fn wasm_bindgen_cli() -> Option<PathBuf> {
    toolchain::WASM_BINDGEN.open()
}

/// The gate: scalar ownership transfers are pinned from the glue, while array element ownership
/// and the explicit copy path are exercised under Node because the transfer occurs inside
/// wasm-bindgen's Rust conversion rather than as a visible `__destroy_into_raw()` JS call. ~keep
///
/// Sabotage checks for this test:
/// - reverting `types_accessors::gen_setter`'s `class_backed_field_type` branch to the by-value
///   `format!` (the pre-#470 emitter) must add
///   `WasmRenderOptions.set palette consumes 'value'` and
///   `WasmRenderOptions.set fallbackPalette consumes 'value'`.
/// - reverting `enums_accessors` to the by-value `value: {ty}` payload setter (the pre-#473
///   emitter) must add `WasmLayer.set palette consumes 'value'`.
/// - reverting the borrowed constructor parameter in `types::gen_new_method` must add
///   `WasmRenderOptions.constructor consumes 'palette'`.
#[test]
fn generated_bindings_do_not_destroy_handles_the_caller_still_owns() {
    let Some(cli) = wasm_bindgen_cli() else { return };

    // A failing run deletes the temp dir before anyone can read the crate that produced the
    // glue. Point this at a real path to keep the whole fixture workspace. ~keep
    let keep = std::env::var_os("ALEF_WASM_FIXTURE_DIR").map(PathBuf::from);
    let tmp = tempfile::tempdir().expect("create fixture workspace dir");
    let root = keep.as_deref().unwrap_or_else(|| tmp.path());

    let manifest = materialize_workspace(root, &cli_crate_version(&cli));
    let module = build_wasm_module(root, &manifest);
    let js = emit_js_glue(&cli, root, &module);
    run_node_ownership_oracle(&cli, root, &module);

    // Anti-vacuity: an empty or truncated fixture would report zero consumed handles and read
    // exactly like a clean one. Every class the assertions below reason about must be here.
    // The panics print the class list rather than the whole glue -- the glue is ~500 lines and
    // buries the finding; `ALEF_WASM_FIXTURE_DIR` keeps the package for reading instead. ~keep
    let declared: Vec<String> = js.lines().filter_map(class_declaration).collect();
    for class in [
        "WasmPalette",
        "WasmRenderOptions",
        "WasmDefaultedOptions",
        "WasmPaletteCollection",
        "WasmLayer",
    ] {
        assert!(
            declared.iter().any(|name| name == class),
            "the emitted glue declares {declared:?}, missing `{class}`, so this run examined less \
             than it claims"
        );
    }

    let consumed = consumed_handles(&js);
    let rendered: Vec<String> = consumed.iter().map(ToString::to_string).collect();
    assert_eq!(
        rendered,
        vec![
            // `#[derive(Default)]` makes every generated parameter `Option<T>`
            // (`shared::config_constructor_parts_inner`), and `Option<&T>` has no
            // `OptionFromWasmAbi` impl, so no parameter here is borrowable. Closed as a
            // documented limitation in alef#479: this is wasm-bindgen's ordinary ownership
            // transfer, the generated rustdoc says so, and `default()` plus the borrowed
            // setters already reach every state the constructor can. These two entries are
            // therefore the intended contract, not a backlog item. ~keep
            "WasmDefaultedOptions.constructor consumes `palette`".to_string(),
            // `Vec<WasmPalette>` lowers through wasm-bindgen's generated `__unwrap`, which
            // necessarily takes ownership of each element. The Node oracle above proves that
            // passing `copyForTransfer()` results preserves the caller's original handles. ~keep
            "WasmPalette.static __unwrap consumes `jsValue`".to_string(),
            // Same wall, reached through the optional parameter of an otherwise-borrowable
            // constructor. ~keep
            "WasmRenderOptions.constructor consumes `fallbackPalette`".to_string(),
        ],
        "the JS glue destroys a handle the caller still owns"
    );

    // The positive half: a borrowed setter has to lower as a plain `__wbg_ptr` read. Checked
    // after the set above so the set is what a regression reports first, and checked at all
    // because "no handle destroyed" is also true of a setter that was never emitted. ~keep
    assert!(
        js.contains("wasm.wasmlayer_set_palette(this.__wbg_ptr, value.__wbg_ptr)"),
        "the tagged-enum payload setter must lower its argument as a borrowed pointer"
    );
    assert!(
        js.contains("wasm.wasmrenderoptions_new(palette.__wbg_ptr,"),
        "the borrowed constructor parameter must lower as a borrowed pointer"
    );
}

fn wasm_ci_step_runs(step: &serde_json::Value) -> bool {
    (step["continue-on-error"].is_null() || step["continue-on-error"] == false)
        && (step["if"].is_null() || step["if"] == true || matches!(step["if"].as_str(), Some("true" | "${{ true }}")))
}

fn wasm_ci_is_provisioned(steps: &[serde_json::Value]) -> bool {
    steps.iter().any(|step| {
        let tools = step["with"]["tools"]
            .as_str()
            .filter(|tools| !tools.trim().is_empty())
            .or_else(|| step["with"]["install-args"].as_str())
            .unwrap_or("");
        wasm_ci_step_runs(step)
            && step["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("xberg-io/actions/setup-mise@"))
            && step["with"]["install"] != false
            && step["with"]["install"] != "false"
            && tools
                .split_whitespace()
                .any(|tool| tool.split(['=', '@']).next() == Some("github:wasm-bindgen/wasm-bindgen"))
    })
}

fn wasm_ci_census_command_matches(command: &str, expected: &[&str]) -> bool {
    let mut words = command.split_whitespace();
    if words.next() != Some("scripts/toolchain-census.sh") {
        return false;
    }
    let arguments: Vec<_> = words.collect();
    if arguments.len() != expected.len() * 2 {
        return false;
    }
    let mut tools = Vec::new();
    for pair in arguments.as_chunks::<2>().0 {
        if pair[0] != "--require" || tools.contains(&pair[1]) {
            return false;
        }
        tools.push(pair[1]);
    }
    expected.iter().all(|tool| tools.contains(tool))
}

fn wasm_ci_census_script_runs(script: &str) -> bool {
    let lines: Vec<_> = script.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    lines.len() == 5
        && lines[0] == "if [ \"$RUNNER_OS\" = \"macOS\" ]; then"
        && lines[2] == "else"
        && lines[4] == "fi"
        && wasm_ci_census_command_matches(lines[1], &["go", "ruby", "swift", "wasm-bindgen"])
        && wasm_ci_census_command_matches(lines[3], &["go", "ruby", "wasm-bindgen"])
}

fn wasm_ci_census_runs(steps: &[serde_json::Value]) -> bool {
    steps.iter().any(|step| {
        let enabled = wasm_ci_step_runs(step) || step["if"] == "${{ !cancelled() }}";
        enabled
            && step["name"] == "Toolchain fixture census"
            && step["shell"] == "bash"
            && (step["continue-on-error"].is_null() || step["continue-on-error"] == false)
            && step["run"].as_str().is_some_and(wasm_ci_census_script_runs)
    })
}

fn wasm_ci_requirement_is_enforced(workflow: &serde_json::Value) -> bool {
    let job = &workflow["jobs"]["test"];
    let Some(steps) = job["steps"].as_array() else {
        return false;
    };
    let Some(run) = steps.iter().position(|step| step["name"] == "Run tests") else {
        return false;
    };
    wasm_ci_step_runs(job)
        && wasm_ci_is_provisioned(&steps[..run])
        && wasm_ci_census_runs(&steps[run + 1..])
        && wasm_ci_step_runs(&steps[run])
        && steps[run]["env"]["ALEF_REQUIRE_WASM_BINDGEN"] == "1"
        && steps[run]["run"].as_str().is_some_and(|run| {
            run.trim().lines().count() == 1
                && run
                    .split_whitespace()
                    .eq(["mise", "run", "test:run", "--", "--no-fail-fast"])
        })
}

#[test]
fn ci_requires_the_wasm_bindgen_cli_for_the_js_glue_oracle() {
    let workflow = serde_saphyr::from_str(include_str!("../../../.github/workflows/ci.yml")).expect("CI workflow YAML");
    assert!(
        wasm_ci_requirement_is_enforced(&workflow),
        "Test must provision and count the required wasm oracle"
    );
}

fn wasm_ci_mutant(workflow: &serde_json::Value, setup: usize, mode: &str) -> serde_json::Value {
    let mut mutant = workflow.clone();
    let steps = mutant["jobs"]["test"]["steps"].as_array_mut().unwrap();
    match mode {
        "remove" => {
            steps.remove(setup);
        }
        "other-job" => {
            let moved = steps.remove(setup);
            mutant["jobs"]["unrelated"]["steps"] = serde_json::json!([moved]);
        }
        "after-tests" => {
            let moved = steps.remove(setup);
            steps.push(moved);
        }
        "disabled" => steps[setup]["if"] = serde_json::json!(false),
        "no-install" => steps[setup]["with"]["install"] = serde_json::json!(false),
        "no-required-mode" => {
            let run = steps.iter_mut().find(|step| step["name"] == "Run tests").unwrap();
            run["env"]["ALEF_REQUIRE_WASM_BINDGEN"] = serde_json::json!("0");
        }
        _ => {
            let census = steps
                .iter_mut()
                .find(|step| step["name"] == "Toolchain fixture census")
                .unwrap();
            if mode == "census" {
                census["if"] = serde_json::json!(false);
            } else {
                let run = census["run"]
                    .as_str()
                    .unwrap()
                    .replacen("--require wasm-bindgen", "", 1);
                census["run"] = serde_json::json!(run);
            }
        }
    }
    mutant
}

#[test]
fn wasm_ci_guard_rejects_displaced_disabled_and_uncounted_oracles() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../.github/workflows/ci.yml")).unwrap();
    let setup = workflow["jobs"]["test"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .position(|step| step["with"]["tools"] == "github:wasm-bindgen/wasm-bindgen")
        .unwrap();
    for mode in [
        "remove",
        "other-job",
        "after-tests",
        "disabled",
        "no-install",
        "census",
        "census-branch",
        "no-required-mode",
    ] {
        let mutant = wasm_ci_mutant(&workflow, setup, mode);
        assert!(!wasm_ci_requirement_is_enforced(&mutant), "accepted {mode}");
    }
}

#[test]
fn wasm_ci_guard_rejects_narrowed_suites() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../.github/workflows/ci.yml")).unwrap();
    for command in [
        "mise run test:run -- --lib no_such_test",
        "mise\nrun test:run -- --no-fail-fast",
        "mise run test:run -- --help",
        "mise run test:run -- --no-fail-fast --lib no_such_test",
        "echo mise run test:run -- --no-fail-fast",
        "mise run test:run -- --no-fail-fast; true",
    ] {
        let mut mutant = workflow.clone();
        let run = mutant["jobs"]["test"]["steps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|step| step["name"] == "Run tests")
            .unwrap();
        run["run"] = serde_json::json!(command);
        assert_ne!(mutant, workflow);
        assert!(
            !wasm_ci_requirement_is_enforced(&mutant),
            "accepted narrowed suite: {command}"
        );
    }
}

#[test]
fn wasm_ci_guard_rejects_unreachable_census_text() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../.github/workflows/ci.yml")).unwrap();
    let census = workflow["jobs"]["test"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["name"] == "Toolchain fixture census")
        .unwrap()["run"]
        .as_str()
        .unwrap();
    for script in [
        format!("cat <<'CENSUS'\n{census}\nCENSUS"),
        format!("if false; then\n{census}\nfi"),
        census.replace("if [ \"$RUNNER_OS\" = \"macOS\" ]; then", "if false; then"),
        census.replacen("--require go", "", 1),
        census.replace("--require go", ""),
    ] {
        let mut mutant = workflow.clone();
        let step = mutant["jobs"]["test"]["steps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|step| step["name"] == "Toolchain fixture census")
            .unwrap();
        step["run"] = serde_json::json!(script);
        assert_ne!(mutant, workflow);
        assert!(
            !wasm_ci_requirement_is_enforced(&mutant),
            "accepted unreachable or incomplete census: {script}"
        );
    }
}

#[test]
fn wasm_ci_guard_rejects_ignored_failures_and_conditional_execution() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../.github/workflows/ci.yml")).unwrap();
    for name in [
        "job",
        "Run tests",
        "Install the wasm-bindgen CLI",
        "Toolchain fixture census",
    ] {
        for (field, value) in [
            ("continue-on-error", serde_json::json!(true)),
            ("if", serde_json::json!(false)),
            ("if", serde_json::json!("${{ runner.os == 'Linux' }}")),
        ] {
            let mut mutant = workflow.clone();
            let job = &mut mutant["jobs"]["test"];
            let target = if name == "job" {
                job
            } else {
                job["steps"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|step| step["name"] == name)
                    .unwrap()
            };
            target[field] = value;
            assert_ne!(mutant, workflow);
            assert!(!wasm_ci_requirement_is_enforced(&mutant), "accepted {name}: {field}");
        }
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::*;

    /// The scanner is the whole measurement, so it gets its own known-answer input: a `free()`
    /// consuming `this` is not a finding, and a setter consuming its argument is. ~keep
    #[test]
    fn the_scanner_reports_argument_consumption_and_ignores_free() {
        let js = "export class WasmThing {\n\
                  \x20   free() {\n\
                  \x20       const ptr = this.__destroy_into_raw();\n\
                  \x20   }\n\
                  \x20   set palette(value) {\n\
                  \x20       var ptr0 = value.__destroy_into_raw();\n\
                  \x20   }\n\
                  }\n";
        let found: Vec<String> = consumed_handles(js).iter().map(ToString::to_string).collect();
        assert_eq!(found, vec!["WasmThing.set palette consumes `value`".to_string()]);
    }
}
