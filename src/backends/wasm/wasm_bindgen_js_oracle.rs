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
//! `wasm-bindgen` CLI, and read which arguments the resulting JS destroys.
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
    let out_dir = root.join("pkg");
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

/// Locate the `wasm-bindgen` CLI, or report that this fixture did not run.
///
/// Routed through the crate-wide [`toolchain::WASM_BINDGEN`] gate, which counts the invocation as
/// attempted and then as executed or skipped; `scripts/toolchain-census.sh --require wasm-bindgen`
/// turns "0 of 1 executed" into a failed CI leg. A bare `eprintln!` skip notice would be
/// invisible here, because `libtest` captures the stderr of a *passing* test. ~keep
fn wasm_bindgen_cli() -> Option<PathBuf> {
    toolchain::WASM_BINDGEN.open()
}

/// The gate: no accessor or constructor in alef's generated wasm bindings may consume a handle
/// the caller still owns, beyond the two positions `wasm-bindgen` gives no way to avoid.
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

    // Anti-vacuity: an empty or truncated fixture would report zero consumed handles and read
    // exactly like a clean one. Every class the assertions below reason about must be here.
    // The panics print the class list rather than the whole glue -- the glue is ~500 lines and
    // buries the finding; `ALEF_WASM_FIXTURE_DIR` keeps the package for reading instead. ~keep
    let declared: Vec<String> = js.lines().filter_map(class_declaration).collect();
    for class in ["WasmPalette", "WasmRenderOptions", "WasmDefaultedOptions", "WasmLayer"] {
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

/// Keeps required mode wired to a job that actually has the CLI on `PATH`.
///
/// `ALEF_REQUIRE_WASM_BINDGEN` alone only turns a silent skip into a permanently red leg when no
/// step installs the binary, and the census `--require wasm-bindgen` is what proves this fixture
/// executed rather than skipped. All three halves have to be present together. ~keep
#[test]
fn ci_requires_the_wasm_bindgen_cli_for_the_js_glue_oracle() {
    let workflow = include_str!("../../../.github/workflows/ci.yml");

    assert!(
        workflow.contains("ALEF_REQUIRE_WASM_BINDGEN: \"1\""),
        "the test job must make a missing wasm-bindgen CLI a hard failure"
    );
    assert!(
        workflow.contains("tool: wasm-bindgen-cli"),
        "`ALEF_REQUIRE_WASM_BINDGEN` needs an explicit CLI install on every leg of the matrix"
    );
    assert!(
        workflow.contains("--require wasm-bindgen"),
        "the toolchain census must require wasm-bindgen so a skipped fixture fails the leg"
    );
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
