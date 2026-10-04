#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]
//! Run the *consumer's* standard toolchain gate over a tree alef just emitted.
//!
//! Every check alef owns today inspects generated text through alef's own eyes: a
//! snapshot, a `contains`, a stage ledger. None of them is the thing a consumer
//! actually runs. In one session three separate downstream gates went red on
//! alef-generated output, from three different emitters, and all three were found by a
//! human reading a failing consumer build:
//!
//! 1. `cargo clippy -- -D warnings` — a redundant pointer cast in generated JNI
//!    (`into_raw()` already returns `*const T`, and the emitter cast it again). A build
//!    failure for the consumer, not a lint nit.
//! 2. `cargo sort --check` — a generated `Cargo.toml` put `[lints.clippy]` ahead of
//!    `[dependencies]`; canonical order puts lints last. Six crates.
//! 3. `poly fmt --check` — `.alef-toml-merge-provenance.toml` emitted with a 4-space
//!    array indent where poly normalises to 2. 931 lines of pure indentation, and a
//!    standing failure the consumer could not fix, because alef rewrote the file on
//!    every run.
//!
//! The three bugs are fixed elsewhere. This file is the gate for the *class*: emit a
//! tree for a synthetic consumer and hand it to the same three tools the consumer runs.
//!
//! A fourth lane, [`cargo_manifest_byte_lane`], has no consumer-side tool behind it: `cargo
//! sort --check` verifies dependency and table ORDER, not byte layout, so a `Cargo.toml`
//! that drifts from what alef emits by nothing but whitespace still passes it, and
//! `Cargo.toml` is deliberately excluded from poly's format pass (see
//! `POLY_FORMAT_EXCLUDES`). No downstream tool covers that gap, so this lane closes it
//! in-house by byte-comparing each manifest against the snapshot alef itself produced.
//!
//! # What makes this gate non-vacuous
//!
//! A gate over generated output has one dominant failure mode, and it is not "the tool
//! reports a false positive" — it is "the tool examined nothing and exited 0". Three
//! ways that happens here, each with a named guard:
//!
//! - **The tools run against alef instead of the emitted tree.** alef's own workspace is
//!   clean, so the job passes while checking the wrong thing. Guarded by
//!   [`assert_emitted_tree_is_isolated`], which is called before every lane and also has
//!   its own test proving it fires.
//! - **Generation produced no Rust manifests, so the cargo lanes had nothing to open.**
//!   Guarded by asserting a non-empty discovered-manifest set in [`EmittedTree::new`].
//! - **A required tool is absent and the lane skips.** There is deliberately no skip
//!   path and no opt-out env var: [`resolve_tools`] fails the test and names the install
//!   command. A gate that downgrades to a pass when its tooling is missing is the exact
//!   defect being eliminated here.
//!
//! The positive proof is the [`Sabotage`] set: one deliberate defect per tool, each
//! shaped like the real bug it stands for, each with a test asserting that tool goes red
//! *and* that the same tree without the defect is green. If a lane ever stops examining
//! the emitted tree, its sabotage test starts failing.
//!
//! # Why the heavy tests are `#[ignore]`
//!
//! `cargo sort` and `poly` are installed by `task setup` and by the gate's CI job, but
//! not by the three-OS `cargo test --workspace` matrix (poly has no Windows tap at all).
//! A hard-failing tool probe in the default suite would break that matrix on every
//! platform, and the predictable repair is to delete the gate.
//!
//! So the lanes are `#[ignore]`d and run explicitly, by `task gate:generated-output` and
//! by the `generated-output-gate` CI job. That trades one risk for another — an ignored
//! test is invisible, and a CI job can be deleted in a one-line diff — so the trade is
//! closed by [`ci_workflow_runs_the_generated_output_gate`], which is *not* ignored,
//! needs no tooling, and fails the ordinary suite if the CI job stops invoking these
//! tests or stops installing what they need. ~keep

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Language coverage
// ---------------------------------------------------------------------------

/// Which lanes a language takes part in.
///
/// The split is not a compromise between cost and coverage — it is the honest shape of
/// the two costs. `poly` and `cargo sort` parse text and never compile, so they run over
/// every language alef emits at negligible cost. `cargo clippy` builds a real dependency
/// graph, so it can only cover languages whose generated crate compiles with nothing but
/// a stock runner's rustc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lanes {
    /// Text lanes only: `poly fmt --check` over the emitted files, and `cargo sort
    /// --check` over any `Cargo.toml` the language emits.
    TextOnly,
    /// Text lanes plus `cargo clippy -- -D warnings`.
    TextAndClippy,
}

/// One row per language. **Adding a language to this gate is this one row** — the
/// fixture's `alef.toml` `languages = [...]` list is joined from `name`, and the clippy
/// lane filters on `lanes`. Nothing else in this file names a language.
struct GateLanguage {
    name: &'static str,
    lanes: Lanes,
    /// Path fragment identifying this language's generated Rust crate directory, used to
    /// pick its manifests out of the emitted tree. Required for [`Lanes::TextAndClippy`]
    /// and empty otherwise.
    ///
    /// The clippy lane has to select manifests rather than take all of them: the emitted
    /// tree also holds Rust crates for php, ruby, elixir and swift, and those are exactly
    /// the ones excluded below for needing a system toolchain. Running clippy over
    /// everything would fail on the runner for reasons that have nothing to do with
    /// alef's emitters. ~keep
    rust_crate_marker: &'static str,
    /// Why a language is text-only. Empty for clippy-covered languages. This exists so a
    /// gap in clippy coverage is a sentence someone had to write, not an absence. ~keep
    clippy_exclusion_reason: &'static str,
}

const GATE_LANGUAGES: &[GateLanguage] = &[
    GateLanguage {
        name: "ffi",
        lanes: Lanes::TextAndClippy,
        rust_crate_marker: "-ffi",
        clippy_exclusion_reason: "",
    },
    GateLanguage {
        name: "python",
        lanes: Lanes::TextAndClippy,
        rust_crate_marker: "-py",
        clippy_exclusion_reason: "",
    },
    GateLanguage {
        name: "node",
        lanes: Lanes::TextAndClippy,
        rust_crate_marker: "-node",
        clippy_exclusion_reason: "",
    },
    GateLanguage {
        name: "wasm",
        lanes: Lanes::TextAndClippy,
        rust_crate_marker: "-wasm",
        clippy_exclusion_reason: "",
    },
    // The emitter that shipped the redundant-cast build failure. It is in the clippy lane
    // for that reason, and it can be: the `jni` crate is pure Rust and needs no JDK to
    // type-check. ~keep
    GateLanguage {
        name: "jni",
        lanes: Lanes::TextAndClippy,
        rust_crate_marker: "-jni",
        clippy_exclusion_reason: "",
    },
    // `jni` is the Rust half of the kotlin_android pairing; both are enabled so the shim
    // crate is generated against the bridge declarations it mirrors. ~keep
    GateLanguage {
        name: "kotlin_android",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "emits Kotlin and Gradle sources; its Rust side is the paired jni crate",
    },
    GateLanguage {
        name: "java",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "emits Java sources; its Rust side is the paired jni crate",
    },
    GateLanguage {
        name: "ruby",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "rb-sys needs libruby headers and a matching interpreter at build time",
    },
    GateLanguage {
        name: "php",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "ext-php-rs needs php-dev headers at build time",
    },
    GateLanguage {
        name: "elixir",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "the rustler NIF crate links against an Erlang runtime",
    },
    GateLanguage {
        name: "swift",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "the swift-bridge crate's build script needs the Swift toolchain",
    },
    GateLanguage {
        name: "go",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "emits cgo and Go sources, no Rust crate of its own",
    },
    GateLanguage {
        name: "csharp",
        lanes: Lanes::TextOnly,
        rust_crate_marker: "",
        clippy_exclusion_reason: "emits C# over the FFI surface, no Rust crate of its own",
    },
];

/// The `languages = [...]` value for the fixture's `alef.toml`, already wrapped.
///
/// Emitted one-per-line because the inline form is far past poly's TOML width and the poly lane
/// then reports the fixture's own `alef.toml` as unformatted alef output. The gate exists to
/// judge what alef emits; it must not fail on the input the test itself writes. ~keep
fn fixture_language_list() -> String {
    let names: Vec<String> = GATE_LANGUAGES
        .iter()
        .map(|language| format!("  \"{}\",", language.name))
        .collect();
    format!("\n{}\n", names.join("\n"))
}

fn clippy_lane_languages() -> Vec<&'static str> {
    GATE_LANGUAGES
        .iter()
        .filter(|language| language.lanes == Lanes::TextAndClippy)
        .map(|language| language.name)
        .collect()
}

// ---------------------------------------------------------------------------
// Fixture: a synthetic consumer, deliberately nobody's real crate
// ---------------------------------------------------------------------------

#[path = "generated_output_downstream_gate/fixture.rs"]
mod fixture;
use fixture::FIXTURE_CARGO_TOML;

#[path = "workflow_job_block/support.rs"]
mod workflow_job_block_support;
use workflow_job_block_support::workflow_job_block;

// ---------------------------------------------------------------------------
// Tooling: present and executable, or the gate fails
// ---------------------------------------------------------------------------

struct GateTool {
    /// The argv[0] the lane invokes.
    program: &'static str,
    /// How the tool is spelled in failure messages — `cargo sort` reads better than the
    /// `cargo` that is actually exec'd.
    display: &'static str,
    /// Arguments that make the tool report its version and exit 0, used to prove the
    /// binary is executable rather than merely a name on `PATH`. A cargo subcommand that
    /// is not installed makes `cargo` itself exit non-zero, so this probe covers the
    /// subcommand and not just cargo. ~keep
    probe: &'static [&'static str],
    /// The arguments that run the tool's check.
    check_args: &'static [&'static str],
    install_hint: &'static str,
}

const CARGO_SORT: GateTool = GateTool {
    program: "cargo",
    display: "cargo sort",
    probe: &["sort", "--version"],
    check_args: &["sort", "--check"],
    install_hint: "cargo install cargo-sort (or `task setup`)",
};

const POLY: GateTool = GateTool {
    program: "poly",
    display: "poly",
    probe: &["--version"],
    check_args: &["fmt", "--check", "."],
    install_hint: "brew install goldziher/tap/poly (or `task setup`)",
};

// `POLY_FMT_LANE_EXCLUSIONS` and `poly_fmt_check_args` live in the `poly_fmt_exclusions`
// submodule -- this file is already over the repo's 1,000-line file cap, and that cap says a
// touched over-limit file must split the touched concern into a smaller module rather than
// grow further. See that module's doc for why each exclusion exists. ~keep
#[path = "generated_output_downstream_gate/poly_fmt_exclusions.rs"]
mod poly_fmt_exclusions;
use poly_fmt_exclusions::poly_fmt_check_args;

// The reverse-direction check for the FFI crate's crate-level clippy allow list (an entry that
// no longer corresponds to any generated pattern) lives in its own submodule for the same
// reason: this file is already over the 1,000-line cap, so a new concern gets a new file
// instead of more lines here. See that module's doc for the full rationale. ~keep
#[path = "generated_output_downstream_gate/ffi_allowlist_gate.rs"]
mod ffi_allowlist_gate;

// The E0004 / `unreachable_patterns` foreign-cfg-enum shape (alef commit f9795aea9) needs its
// own fixture crate and two compile-level sabotages; see that module's doc for why this is not
// inline here either. ~keep
#[path = "generated_output_downstream_gate/foreign_cfg_gate.rs"]
mod foreign_cfg_gate;

const CARGO: GateTool = GateTool {
    program: "cargo",
    display: "cargo clippy",
    probe: &["clippy", "--version"],
    check_args: &["clippy", "--all-targets", "--", "-D", "warnings"],
    install_hint: "rustup component add clippy",
};

/// Prove every tool a lane needs is installed and runnable, or fail the test naming all
/// of them at once.
///
/// There is no `ALEF_..._ALLOW_MISSING_TOOLS` escape and no `return` that leaves the test
/// green. A gate whose tooling is absent has examined nothing, and a run that examined
/// nothing must not be reportable as a healthy one — that equivalence is the whole reason
/// this file exists. ~keep
fn resolve_tools(tools: &[&GateTool]) {
    let mut missing = Vec::new();
    for tool in tools {
        let runnable = Command::new(tool.program)
            .args(tool.probe)
            .output()
            .is_ok_and(|output| output.status.success());
        if !runnable {
            missing.push(format!(
                "  {} — not installed or not runnable; {}",
                tool.display, tool.install_hint
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the generated-output gate cannot run without its downstream tooling.\n\
         This is a failure, not a skip: with these tools absent the gate examines nothing.\n{}",
        missing.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The isolation guard
// ---------------------------------------------------------------------------

/// Refuse to run a lane unless the tree under test is provably not alef's own workspace.
///
/// This is the guard the brief for this gate is built around, so it checks the property
/// three ways rather than one:
///
/// 1. The emitted root is neither alef's manifest directory nor anywhere beneath it.
/// 2. alef's manifest directory is not beneath the emitted root either — the containment
///    has to fail in both directions, or a gate pointed at `/` would pass step 1.
/// 3. No ancestor of the emitted root holds a `Cargo.toml`. This is the one that catches
///    the realistic accident: a tempdir nested inside some cargo workspace makes
///    `cargo clippy --manifest-path` pull that parent workspace in, and the lane then
///    lints crates nobody generated while still reporting on the emitted ones.
///
/// Returns the canonical emitted root so callers cannot go on using an uncanonicalised
/// path that would compare differently — on macOS `/var` is a symlink to `/private/var`,
/// and every comparison here is textual.
fn assert_emitted_tree_is_isolated(emitted_root: &Path) -> PathBuf {
    let emitted = emitted_root
        .canonicalize()
        .unwrap_or_else(|error| panic!("canonicalize emitted root {}: {error}", emitted_root.display()));
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .expect("canonicalize alef manifest directory");

    assert!(
        emitted != repo_root && !emitted.starts_with(&repo_root),
        "the gate would have run over alef's own workspace instead of an emitted tree.\n\
         emitted root: {}\n  alef root:  {}",
        emitted.display(),
        repo_root.display()
    );
    assert!(
        !repo_root.starts_with(&emitted),
        "the emitted root contains alef's own workspace, so the lanes would lint alef too.\n\
         emitted root: {}\n  alef root:  {}",
        emitted.display(),
        repo_root.display()
    );

    for ancestor in emitted.ancestors().skip(1) {
        assert!(
            !ancestor.join("Cargo.toml").is_file(),
            "the emitted tree is nested inside a cargo workspace rooted at {}.\n\
             Cargo commands would resolve that parent manifest and lint crates this gate \
             never generated.\n  emitted root: {}",
            ancestor.display(),
            emitted.display()
        );
    }

    emitted
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Which deliberate defect, if any, is injected into the emitted tree.
///
/// Each variant reproduces the *shape* of one of the three real downstream failures, and
/// each is paired with a test asserting the matching tool goes red. Injection happens
/// after generation, on the emitted files: the emitters themselves are fixed, so the only
/// honest way to prove the lanes can still see this class is to put the defect back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sabotage {
    /// The tree exactly as alef emits it.
    None,
    /// Put a `[lints.clippy]` table ahead of `[dependencies]` in an emitted manifest.
    /// Canonical order puts lints last, so `cargo sort --check` must reject it.
    MisorderedCargoTable,
    /// Re-indent an emitted TOML array to four spaces, where poly normalises to two.
    /// Stands for the provenance-manifest indent that the consumer could not fix.
    WideTomlArrayIndent,
    /// Widen the spacing around every `key = value` pair's `=` in an emitted `Cargo.toml`,
    /// leaving table order and dependency key order untouched. `cargo sort --check` is
    /// built on `toml_edit::Table::sort_values`, which reorders entries without touching
    /// an already-correctly-ordered entry's own decor -- so this is invisible to it, and
    /// `Cargo.toml` is also excluded from poly's format pass (see
    /// `scaffold::languages::poly::POLY_FORMAT_EXCLUDES`, which hands `Cargo.toml` to
    /// cargo-sort specifically to avoid the two tools fighting). Stands for the class of
    /// bug neither downstream tool can see: a manifest that drifted from what alef emits
    /// by nothing but whitespace. ~keep
    CargoManifestIndentDrift,
    /// Add a redundant pointer cast to an emitted Rust file, the shape that broke the
    /// consumer's `-D warnings` build.
    RedundantPointerCast,
}

struct EmittedTree {
    root: PathBuf,
    /// Every `Cargo.toml` alef emitted, excluding the fixture's two hand-authored manifests
    /// (`toolkit`'s own root and the sibling `foreign_core` crate -- see `foreign_cfg_gate`).
    manifests: Vec<PathBuf>,
    /// Every `.toml` alef emitted, the poly lane's evidence surface.
    toml_files: Vec<PathBuf>,
    /// Each manifest's exact bytes at the moment alef emitted it, captured before any
    /// [`Sabotage`] runs. This is the reference [`cargo_manifest_byte_lane`] diffs
    /// against -- the "committed manifest" half of the comparison, sourced from the same
    /// `alef` binary run that produced the tree rather than from a hand-described layout.
    /// ~keep
    manifest_snapshots: BTreeMap<PathBuf, String>,
    _workspace: tempfile::TempDir,
}

impl EmittedTree {
    fn new(workspace: tempfile::TempDir, root: PathBuf) -> Self {
        let core_manifest = root.join("Cargo.toml");
        let foreign_core_manifest = root.join("foreign_core/Cargo.toml");
        let mut manifests = Vec::new();
        let mut toml_files = Vec::new();
        for path in WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
            .map(walkdir::DirEntry::into_path)
            .filter(|path| path.is_file())
        {
            if path.extension().is_some_and(|extension| extension == "toml") {
                toml_files.push(path.clone());
            }
            let is_hand_authored = path == core_manifest || path == foreign_core_manifest;
            if path.file_name().is_some_and(|name| name == "Cargo.toml") && !is_hand_authored {
                manifests.push(path);
            }
        }
        manifests.sort();
        toml_files.sort();

        // A cargo lane over an empty manifest set exits 0 having opened no files, and
        // reads identically to a healthy run. Fail here instead, where the message can
        // say which of the two happened. ~keep
        assert!(
            !manifests.is_empty(),
            "generation emitted no Cargo.toml outside the fixture's own crate, so the cargo \
             lanes would examine nothing.\n  emitted root: {}",
            root.display()
        );

        // The negative half of "it lints the emitted tree, not alef": no manifest in the
        // examined set may be alef's. The positive half is the sabotage tests. ~keep
        let mut manifest_snapshots = BTreeMap::new();
        for manifest in &manifests {
            let text = std::fs::read_to_string(manifest).unwrap_or_default();
            assert!(
                !text.contains("name = \"alef\""),
                "the examined manifest set contains alef's own package: {}",
                manifest.display()
            );
            manifest_snapshots.insert(manifest.clone(), text);
        }

        Self {
            root,
            manifests,
            toml_files,
            manifest_snapshots,
            _workspace: workspace,
        }
    }

    fn manifest_dirs(&self) -> Vec<&Path> {
        self.manifests.iter().filter_map(|manifest| manifest.parent()).collect()
    }
}

/// Emit the tree by running the real `alef` binary over the fixture.
///
/// Deliberately the binary and not the library stages, even though driving
/// `alef::cli::pipeline` in-process (the way `pipeline_regeneration_gate` does) would be
/// faster and would need no build. A gate over generated output is only worth its runtime
/// if the tree it examines is the tree a consumer gets, and a hand-picked list of library
/// stages is a second, quietly diverging definition of "what alef emits" — the ordering
/// and merge behaviour that produced two of the three bugs lives in the CLI's write path,
/// not in the backends. Running `alef` means the gate cannot disagree with production
/// about what production does.
///
/// `generate` then `scaffold`, rather than `all`: those two produce the tree, while `all`
/// additionally shells out to per-language build and format toolchains that no runner has
/// installed in full. Both inherit `current_dir`, which is how the emitted root is kept
/// inside the tempdir without any global CWD mutation.
fn emit_tree(sabotage: Sabotage) -> EmittedTree {
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    let root = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());
    foreign_cfg_gate::write_fixture_workspace(&root);
    assert_emitted_tree_is_isolated(&root);

    for stage in ["generate", "scaffold"] {
        let outcome = run_tool(env!("CARGO_BIN_EXE_alef"), &[stage], &root);
        assert!(
            outcome.passed,
            "`alef {stage}` failed over the gate fixture, so there is no emitted tree to check:\n\
             --- {} ---\n{}",
            outcome.command, outcome.output
        );
    }

    let tree = EmittedTree::new(workspace, root);
    inject(&tree, sabotage);
    tree
}

/// Put one of the three historical defects back into the emitted tree.
fn inject(tree: &EmittedTree, sabotage: Sabotage) {
    match sabotage {
        Sabotage::None => {}
        Sabotage::MisorderedCargoTable => {
            let manifest = tree
                .manifests
                .first()
                .expect("manifest set is non-empty by EmittedTree::new");
            let text = std::fs::read_to_string(manifest).expect("read manifest to sabotage");
            let misordered = format!("[lints.clippy]\nredundant_clone = \"deny\"\n\n{text}");
            std::fs::write(manifest, misordered).expect("write misordered manifest");
        }
        Sabotage::WideTomlArrayIndent => {
            // Skip anything under a dot-directory: alef's own `.alef/` cache lives in the
            // emitted tree, poly's discovery does not descend into it, and a sabotage
            // planted somewhere the tool never looks would fail this proof for a reason
            // that has nothing to do with the lane. ~keep
            //
            // Also skip `Cargo.toml`: it is excluded from poly's own format pass at any depth
            // (`scaffold::languages::poly::POLY_FORMAT_EXCLUDES`, cargo-sort owns it instead --
            // see `Sabotage::CargoManifestIndentDrift`'s doc), and it sorts alphabetically
            // ahead of every other emitted TOML at the tree root. Without this, the sabotage
            // landed in a file `poly fmt --check` never looks at, and the lane's own
            // anti-vacuity proof passed while examining nothing -- the exact defect this
            // sabotage exists to catch, just one level up. ~keep
            //
            // Matched against the path *relative to the emitted root*: `tempfile` names
            // its directories `.tmpXXXXXX`, so testing the absolute path would reject
            // every file in the tree and leave nothing to sabotage. ~keep
            let target = tree
                .toml_files
                .iter()
                .find(|path| {
                    path.file_name().is_some_and(|name| name != "Cargo.toml")
                        && path.strip_prefix(&tree.root).is_ok_and(|relative| {
                            !relative
                                .components()
                                .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
                        })
                })
                .expect("emitted tree contains a TOML file outside a dot-directory and not named Cargo.toml");
            let text = std::fs::read_to_string(target).expect("read toml to sabotage");
            let widened = format!("{text}\n[gate_sabotage]\nvalues = [\n    \"a\",\n    \"b\",\n]\n");
            std::fs::write(target, widened).expect("write wide-indent toml");
        }
        Sabotage::CargoManifestIndentDrift => {
            let manifest = tree
                .manifests
                .first()
                .expect("manifest set is non-empty by EmittedTree::new");
            let text = std::fs::read_to_string(manifest).expect("read manifest to sabotage");
            let drifted = widen_key_value_spacing(&text);
            assert_ne!(
                drifted,
                text,
                "the whitespace sabotage produced no change -- {} has no `key = value` line to widen",
                manifest.display()
            );
            std::fs::write(manifest, drifted).expect("write indent-drifted manifest");
        }
        Sabotage::RedundantPointerCast => {
            let target = emitted_rust_source(tree).expect("emitted tree contains a Rust source file");
            let text = std::fs::read_to_string(&target).expect("read rust source to sabotage");
            let with_cast = format!("{text}\n{REDUNDANT_CAST_SNIPPET}");
            std::fs::write(&target, with_cast).expect("write redundant-cast source");
        }
    }
}

/// The shape of the JNI regression: `into_raw()` already yields `*const Handle`, and the
/// emitter cast it to `*const Handle` again before widening. `clippy::unnecessary_cast`
/// denies the middle hop under `-D warnings`. ~keep
const REDUNDANT_CAST_SNIPPET: &str = r"
#[allow(dead_code)]
pub struct GateSabotageHandle {
    value: u64,
}

#[allow(dead_code)]
pub fn gate_sabotage_into_handle(value: Box<GateSabotageHandle>) -> i64 {
    let raw = Box::into_raw(value) as *const GateSabotageHandle;
    raw as *const GateSabotageHandle as i64
}
";

/// Widen the spacing around every top-level `key = value` pair's `=` from one space to
/// two, leaving everything else -- including line order and every key and value -- byte
/// identical. Built by transforming alef's own correct output rather than hand-writing a
/// "wrong" manifest, so it cannot accidentally diverge from real emitted text in some
/// other way that would make the sabotage test pass for the wrong reason.
///
/// Restricted to lines that split cleanly on `" = "` and are not a table header or blank
/// line, which keeps this from reaching into a multi-line value's continuation lines --
/// none exist in an alef-emitted `Cargo.toml` today, but the restriction costs nothing and
/// removes the risk if one ever does. ~keep
fn widen_key_value_spacing(manifest: &str) -> String {
    let mut widened: String = manifest
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('[') || trimmed.starts_with('#') {
                return line.to_owned();
            }
            let indent = &line[..line.len() - trimmed.len()];
            match trimmed.split_once(" = ") {
                Some((key, value)) => format!("{indent}{key}  =  {value}"),
                None => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    widened.push('\n');
    widened
}

/// Pick a Rust file the *clippy* lane will actually open.
///
/// Restricted to the clippy lane's own crates on purpose: dropping the cast into a php or
/// ruby crate would leave the sabotage test failing for the honest reason that clippy
/// never looks there, which reads as "the lane is broken" instead of "the lane does not
/// cover that language". ~keep
/// A source file the redundant-cast sabotage can actually be detected in.
///
/// Skips any file that allows `clippy::unnecessary_cast` at crate level. The emitted FFI crate's
/// `lib.rs` -- the alphabetically first candidate, and so the one this used to return every time
/// -- carries exactly that allow, so the sabotage compiled clean and the lane's self-check
/// reported success while proving nothing. A lane that cannot fail is the thing this file exists
/// to prevent, so an all-candidates-allow-it tree is a hard error rather than a skip. ~keep
fn emitted_rust_source(tree: &EmittedTree) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = clippy_manifest_dirs(tree)
        .iter()
        .flat_map(|dir| WalkDir::new(dir).into_iter().filter_map(Result::ok))
        .map(walkdir::DirEntry::into_path)
        .filter(|path| path.is_file() && path.extension().is_some_and(|extension| extension == "rs"))
        .filter(|path| path.components().any(|component| component.as_os_str() == "src"))
        .collect();
    candidates.sort();

    let allows_the_lint = |path: &PathBuf| {
        std::fs::read_to_string(path).is_ok_and(|body| {
            body.lines()
                .filter(|line| line.starts_with("#!["))
                .any(|line| line.contains("clippy::unnecessary_cast"))
        })
    };
    let lintable: Vec<PathBuf> = candidates
        .iter()
        .filter(|path| !allows_the_lint(path))
        .cloned()
        .collect();
    assert!(
        !lintable.is_empty() || candidates.is_empty(),
        "every emitted Rust source allows `clippy::unnecessary_cast` at crate level, so the \
         redundant-cast sabotage cannot be detected anywhere and the clippy lane's self-check \
         would pass without examining anything. Candidates:\n{}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    lintable.into_iter().next()
}

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

/// What one tool invocation did, kept as data so a failing lane can report the exact
/// command and the directory it ran in rather than just a boolean.
struct LaneOutcome {
    command: String,
    passed: bool,
    output: String,
}

fn run_tool(program: &str, args: &[&str], cwd: &Path) -> LaneOutcome {
    let command = format!("{program} {} (in {})", args.join(" "), cwd.display());
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("running `{command}`: {error}"));
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    LaneOutcome {
        command,
        passed: output.status.success(),
        output: combined,
    }
}

/// `cargo sort --check` over every emitted manifest directory.
///
/// Per-directory rather than `--workspace`, so the emitted crates do not have to be
/// members of a workspace the fixture would then have to declare, and so a failure names
/// the crate.
fn cargo_sort_lane(tree: &EmittedTree) -> Vec<LaneOutcome> {
    let root = assert_emitted_tree_is_isolated(&tree.root);
    assert_eq!(root, tree.root, "isolation guard returned a different root");
    tree.manifest_dirs()
        .into_iter()
        .map(|dir| run_tool(CARGO_SORT.program, CARGO_SORT.check_args, dir))
        .collect()
}

/// Byte-compare every emitted manifest against the snapshot [`EmittedTree::new`] took at
/// the moment alef emitted it.
///
/// This is the gate `cargo sort --check` cannot be: cargo-sort verifies dependency and
/// table ORDER by re-sorting with `toml_edit`, which reorders entries without rewriting an
/// already-correctly-ordered entry's own decor, so a manifest that differs from what alef
/// emitted by nothing but whitespace still parses to the same order and passes `--check`.
/// `Cargo.toml` is also carved out of poly's format pass on purpose (see
/// `POLY_FORMAT_EXCLUDES` in `scaffold::languages::poly`), so no downstream tool this gate
/// runs examines a manifest's byte layout at all. This lane needs no external tool -- the
/// reference it diffs against comes from the same `alef` binary run that produced the
/// tree, not from a hand-described expected layout, so it cannot drift out of sync with a
/// future emitter change the way a golden fixture would. ~keep
fn cargo_manifest_byte_lane(tree: &EmittedTree) -> Vec<LaneOutcome> {
    tree.manifests
        .iter()
        .map(|manifest| {
            let command = format!("byte-compare {} against alef's emitted bytes", manifest.display());
            let current = std::fs::read_to_string(manifest).unwrap_or_default();
            let original = tree
                .manifest_snapshots
                .get(manifest)
                .unwrap_or_else(|| panic!("no emitted-bytes snapshot recorded for {}", manifest.display()));
            match first_difference(original, &current) {
                None => LaneOutcome {
                    command,
                    passed: true,
                    output: String::new(),
                },
                Some((line_number, expected, actual)) => LaneOutcome {
                    command,
                    passed: false,
                    output: format!(
                        "{} differs from alef's emitted bytes at line {line_number}:\n  \
                         emitted:  {expected:?}\n  on disk:  {actual:?}",
                        manifest.display()
                    ),
                },
            }
        })
        .collect()
}

/// The 1-based line number and the two lines' text at the first point `expected` and
/// `actual` diverge, or `None` if they are identical. Compares by line rather than by
/// whole-string equality so a failure message can name the exact line instead of dumping
/// the entire manifest twice.
fn first_difference(expected: &str, actual: &str) -> Option<(usize, String, String)> {
    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    let mut line_number = 0usize;
    loop {
        line_number += 1;
        match (expected_lines.next(), actual_lines.next()) {
            (None, None) => return None,
            (expected_line, actual_line) if expected_line == actual_line => {}
            (expected_line, actual_line) => {
                return Some((
                    line_number,
                    expected_line
                        .unwrap_or("<no line -- file is shorter than emitted>")
                        .to_owned(),
                    actual_line
                        .unwrap_or("<no line -- file is shorter than emitted>")
                        .to_owned(),
                ));
            }
        }
    }
}

/// The emitted manifests belonging to clippy-lane languages, keyed off
/// [`GateLanguage::rust_crate_marker`].
///
/// Asserts every clippy-lane language matched at least one manifest. That assertion is
/// the anti-vacuity guard for this lane specifically: if a backend's output directory is
/// ever renamed, the markers stop matching, the selection silently narrows to nothing,
/// and `cargo clippy` over an empty set exits 0 — a green lane that compiled no generated
/// code at all. ~keep
fn clippy_manifest_dirs(tree: &EmittedTree) -> Vec<PathBuf> {
    let mut selected = Vec::new();
    let mut unmatched = Vec::new();
    for language in GATE_LANGUAGES
        .iter()
        .filter(|language| language.lanes == Lanes::TextAndClippy)
    {
        assert!(
            !language.rust_crate_marker.is_empty(),
            "language `{}` is in the clippy lane but declares no rust_crate_marker, so its \
             crate cannot be located in the emitted tree",
            language.name
        );
        let matches: Vec<PathBuf> = tree
            .manifest_dirs()
            .into_iter()
            .filter(|dir| {
                dir.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(language.rust_crate_marker))
            })
            .map(Path::to_path_buf)
            .collect();
        if matches.is_empty() {
            unmatched.push(format!("  {} (marker `{}`)", language.name, language.rust_crate_marker));
        }
        selected.extend(matches);
    }

    assert!(
        unmatched.is_empty(),
        "no emitted crate directory matched these clippy-lane languages, so clippy would \
         examine nothing for them:\n{}\nemitted crate directories: {:?}",
        unmatched.join("\n"),
        tree.manifest_dirs()
            .iter()
            .filter_map(|dir| dir.file_name())
            .collect::<Vec<_>>()
    );

    selected.sort();
    selected.dedup();
    selected
}

/// `poly fmt --check` over the whole emitted tree, in one invocation.
///
/// poly covers every language alef emits, not just the Rust-bearing ones, which is why
/// this lane is the one that runs unconditionally over the full `GATE_LANGUAGES` set.
fn poly_fmt_lane(tree: &EmittedTree) -> LaneOutcome {
    let root = assert_emitted_tree_is_isolated(&tree.root);
    let args = poly_fmt_check_args();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_tool(POLY.program, &arg_refs, &root)
}

/// `cargo clippy -- -D warnings` over each emitted manifest, for the clippy-lane
/// languages only.
fn clippy_lane(tree: &EmittedTree) -> Vec<LaneOutcome> {
    let root = assert_emitted_tree_is_isolated(&tree.root);
    assert_eq!(root, tree.root, "isolation guard returned a different root");
    clippy_manifest_dirs(tree)
        .iter()
        .map(|dir| run_tool(CARGO.program, CARGO.check_args, dir))
        .collect()
}

fn report(outcomes: &[LaneOutcome]) -> String {
    outcomes
        .iter()
        .filter(|outcome| !outcome.passed)
        .map(|outcome| format!("--- {} ---\n{}", outcome.command, outcome.output))
        .collect::<Vec<_>>()
        .join("\n")
}

fn any_failed(outcomes: &[LaneOutcome]) -> bool {
    outcomes.iter().any(|outcome| !outcome.passed)
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs cargo-sort; run via `task gate:generated-output` or the CI gate job"]
fn emitted_tree_passes_cargo_sort() {
    resolve_tools(&[&CARGO_SORT]);
    let tree = emit_tree(Sabotage::None);
    let outcomes = cargo_sort_lane(&tree);
    assert!(
        !any_failed(&outcomes),
        "`cargo sort --check` rejected alef-generated manifests:\n{}",
        report(&outcomes)
    );
}

#[test]
#[ignore = "needs poly; run via `task gate:generated-output` or the CI gate job"]
fn emitted_tree_passes_poly_fmt() {
    resolve_tools(&[&POLY]);
    let tree = emit_tree(Sabotage::None);
    let outcome = poly_fmt_lane(&tree);
    assert!(
        outcome.passed,
        "`poly fmt --check` rejected alef-generated output:\n--- {} ---\n{}",
        outcome.command, outcome.output
    );
}

#[test]
#[ignore = "regenerates a full emitted tree via the alef binary; run via `task gate:generated-output` \
            or the CI gate job"]
fn emitted_tree_passes_cargo_manifest_byte_lane() {
    let tree = emit_tree(Sabotage::None);
    let outcomes = cargo_manifest_byte_lane(&tree);
    assert!(
        !any_failed(&outcomes),
        "an untouched emitted tree must byte-match its own snapshot, or this lane is not a \
         faithful byte comparison:\n{}",
        report(&outcomes)
    );
}

#[test]
#[ignore = "compiles the emitted crates; run via the CI gate job"]
fn emitted_tree_passes_clippy() {
    resolve_tools(&[&CARGO]);
    let tree = emit_tree(Sabotage::None);
    let outcomes = clippy_lane(&tree);
    assert!(
        !any_failed(&outcomes),
        "`cargo clippy -- -D warnings` rejected alef-generated crates:\n{}",
        report(&outcomes)
    );
}

// ---------------------------------------------------------------------------
// Anti-vacuity: each lane, shown going red
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs cargo-sort; run via `task gate:generated-output` or the CI gate job"]
fn cargo_sort_lane_catches_a_misordered_table() {
    resolve_tools(&[&CARGO_SORT]);
    let clean = emit_tree(Sabotage::None);
    let control = cargo_sort_lane(&clean);
    assert!(
        !any_failed(&control),
        "the control tree must be green, or the sabotage proves nothing:\n{}",
        report(&control)
    );

    let sabotaged = emit_tree(Sabotage::MisorderedCargoTable);
    let outcomes = cargo_sort_lane(&sabotaged);
    assert!(
        any_failed(&outcomes),
        "a `[lints.clippy]` table ahead of `[dependencies]` did not fail `cargo sort --check`, \
         so this lane is not examining the emitted manifests"
    );
}

#[test]
#[ignore = "needs poly; run via `task gate:generated-output` or the CI gate job"]
fn poly_fmt_lane_catches_a_wide_toml_array_indent() {
    resolve_tools(&[&POLY]);
    let clean = emit_tree(Sabotage::None);
    let clean_outcome = poly_fmt_lane(&clean);
    assert!(
        clean_outcome.passed,
        "the control tree must be green, or the sabotage proves nothing:\n{}",
        clean_outcome.output
    );

    let sabotaged = emit_tree(Sabotage::WideTomlArrayIndent);
    let outcome = poly_fmt_lane(&sabotaged);
    assert!(
        !outcome.passed,
        "a four-space TOML array indent did not fail `poly fmt --check`, so this lane is not \
         examining the emitted TOML"
    );
}

/// The test this whole lane exists for: a manifest that differs from what alef emitted by
/// nothing but indentation must fail the gate -- and, to prove that gap was real and not
/// already covered, `cargo sort --check` must still pass the exact same sabotaged tree.
#[test]
#[ignore = "needs cargo-sort; run via `task gate:generated-output` or the CI gate job"]
fn cargo_manifest_byte_lane_catches_indentation_only_drift() {
    resolve_tools(&[&CARGO_SORT]);

    let clean = emit_tree(Sabotage::None);
    let clean_outcomes = cargo_manifest_byte_lane(&clean);
    assert!(
        !any_failed(&clean_outcomes),
        "the control tree must be byte-identical to its own snapshot, or the sabotage proves \
         nothing:\n{}",
        report(&clean_outcomes)
    );

    let sabotaged = emit_tree(Sabotage::CargoManifestIndentDrift);

    let byte_outcomes = cargo_manifest_byte_lane(&sabotaged);
    assert!(
        any_failed(&byte_outcomes),
        "widening the spacing around every `key = value` pair's `=` did not fail the byte \
         comparison, so this lane is not examining the emitted manifest's bytes"
    );

    let sort_outcomes = cargo_sort_lane(&sabotaged);
    assert!(
        !any_failed(&sort_outcomes),
        "`cargo sort --check` rejected an indentation-only change, so it is not the blind spot \
         this lane exists to cover -- update the doc comments on `Sabotage::CargoManifestIndentDrift` \
         and `cargo_manifest_byte_lane` if cargo-sort's behaviour has changed:\n{}",
        report(&sort_outcomes)
    );
}

#[test]
#[ignore = "compiles the emitted crates; run via the CI gate job"]
fn clippy_lane_catches_a_redundant_pointer_cast() {
    resolve_tools(&[&CARGO]);
    let clean = emit_tree(Sabotage::None);
    let control = clippy_lane(&clean);
    assert!(
        !any_failed(&control),
        "the control tree must be green, or the sabotage proves nothing:\n{}",
        report(&control)
    );

    let sabotaged = emit_tree(Sabotage::RedundantPointerCast);
    let outcomes = clippy_lane(&sabotaged);
    assert!(
        any_failed(&outcomes),
        "a redundant pointer cast did not fail `cargo clippy -- -D warnings`, so this lane is \
         not examining the emitted Rust"
    );
}

// ---------------------------------------------------------------------------
// Anti-vacuity: the isolation guard, shown going red
// ---------------------------------------------------------------------------

/// The guard is the reason this whole gate is trustworthy, so it gets its own proof
/// rather than being taken on faith. It needs no external tooling and is therefore not
/// ignored: it runs in the ordinary suite, on every platform.
#[test]
fn isolation_guard_rejects_alefs_own_workspace() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| assert_emitted_tree_is_isolated(&repo_root)));
    assert!(
        result.is_err(),
        "the isolation guard accepted alef's own workspace as an emitted tree — the failure \
         mode this gate exists to prevent"
    );
}

#[test]
fn isolation_guard_rejects_a_tree_nested_in_a_cargo_workspace() {
    let outer = tempfile::tempdir().expect("create outer workspace");
    std::fs::write(outer.path().join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write outer manifest");
    let nested = outer.path().join("nested");
    std::fs::create_dir_all(&nested).expect("create nested tree");

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| assert_emitted_tree_is_isolated(&nested)));
    assert!(
        result.is_err(),
        "the isolation guard accepted a tree whose parent holds a Cargo.toml; cargo would \
         resolve that parent manifest and lint crates the gate never generated"
    );
}

#[test]
fn isolation_guard_accepts_a_standalone_temp_tree() {
    let workspace = tempfile::tempdir().expect("create workspace");
    let accepted = assert_emitted_tree_is_isolated(workspace.path());
    assert!(
        accepted.is_absolute(),
        "the guard must return a canonical absolute root, got {}",
        accepted.display()
    );
}

// ---------------------------------------------------------------------------
// Anti-vacuity: the lanes cannot be silently switched off
// ---------------------------------------------------------------------------

/// Every lane above is `#[ignore]`d, so `cargo test --workspace` does not run any of
/// them. Their authority comes entirely from one CI job, and a CI job is a one-line diff
/// away from not existing.
///
/// This test is the load-bearing half of that trade: it needs no tooling, is not ignored,
/// and asserts the job still invokes this test binary with `--ignored` and still installs
/// what the lanes need. Delete the job or drop `--ignored` and the ordinary suite goes
/// red on every platform. ~keep
/// The name of the CI job, in one place, since both this test and the workflow depend on
/// it matching.
const GATE_JOB: &str = "generated-output-gate";

#[test]
fn ci_workflow_runs_the_generated_output_gate() {
    let workflow_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
    let workflow = std::fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let block = workflow_job_block(&workflow, GATE_JOB).unwrap_or_else(|| {
        panic!(
            "{} has no `{GATE_JOB}` job. Every lane in this file is #[ignore]d, so with that job \
             gone nothing runs the downstream gate at all.",
            workflow_path.display()
        )
    });

    assert_gate_job_installs_poly(&block, &workflow_path);

    let required: &[(&str, &str)] = &[
        (
            "--test generated_output_downstream_gate",
            "the gate job must invoke this test binary by name",
        ),
        (
            "--ignored",
            "every lane in this file is #[ignore]d, so the gate job must pass --ignored or it \
             runs none of them",
        ),
        (
            "cargo-sort",
            "the gate job must install cargo-sort, or the cargo sort lane fails on a missing tool",
        ),
        (
            "clippy",
            "the gate job must install the clippy component, or the clippy lane fails on a \
             missing tool",
        ),
    ];

    let missing: Vec<String> = required
        .iter()
        .filter(|(needle, _)| !block.contains(needle))
        .map(|(needle, reason)| format!("  `{needle}` — {reason}"))
        .collect();

    assert!(
        missing.is_empty(),
        "the `{GATE_JOB}` job in {} no longer wires up the generated-output gate:\n{}\n\
         --- job block as parsed ---\n{block}",
        workflow_path.display(),
        missing.join("\n")
    );
}

#[test]
fn task_and_ci_serialize_ignored_generated_output_lanes() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let taskfile_path = repo_root.join("Taskfile.yml");
    let taskfile = std::fs::read_to_string(&taskfile_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", taskfile_path.display()));
    let workflow_path = repo_root.join(".github/workflows/ci.yml");
    let workflow = std::fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let ignored_task_invocations: Vec<&str> = taskfile
        .lines()
        .filter(|line| line.contains("generated_output_downstream_gate") && line.contains("--ignored"))
        .collect();
    assert_eq!(
        ignored_task_invocations.len(),
        4,
        "unexpected Taskfile gate invocation count"
    );
    assert!(
        ignored_task_invocations
            .iter()
            .all(|line| line.contains("--test-threads=1")),
        "{} must serialize every ignored generated-output invocation",
        taskfile_path.display()
    );

    let serialized_ci_invocation =
        "run: cargo test --test generated_output_downstream_gate -- --ignored --nocapture --test-threads=1";
    assert!(
        workflow.lines().any(|line| line.trim() == serialized_ci_invocation),
        "{} must serialize the ignored generated-output lanes",
        workflow_path.display()
    );
}

/// Whether `block` has an actual `uses:` step line installing `Goldziher/poly`.
///
/// Line-scoped rather than a whole-block `contains("Goldziher/poly")`: the surrounding
/// comments on the `Install poly` step mention `Goldziher/poly` several times in prose (the
/// pin rationale, the SHA-vs-tag explanation, the version-mismatch error message), so a bare
/// substring search would still pass with the real `uses:` line deleted. Only a line whose
/// trimmed text starts with `uses: Goldziher/poly@` counts. ~keep
fn gate_job_installs_poly(block: &str) -> bool {
    const USES_PREFIX: &str = "uses: Goldziher/poly@";
    block.lines().any(|line| line.trim_start().starts_with(USES_PREFIX))
}

/// Panics with a job-block dump if [`gate_job_installs_poly`] does not hold. Split out of
/// [`ci_workflow_runs_the_generated_output_gate`] so fixing this needle did not grow that
/// (already-long, pre-existing) test function any further.
fn assert_gate_job_installs_poly(block: &str, workflow_path: &Path) {
    assert!(
        gate_job_installs_poly(block),
        "the `{GATE_JOB}` job in {} must have a `uses: Goldziher/poly@...` step, not just prose \
         mentioning `Goldziher/poly` -- the poly fmt lane fails on a missing tool if the real \
         install step is gone:\n\
         --- job block as parsed ---\n{block}",
        workflow_path.display()
    );
}

/// A language row that opts out of the clippy lane has to say why.
///
/// Without this, the cheap way to make a clippy failure go away is to flip a row to
/// `TextOnly`, and the coverage hole it opens leaves no trace anywhere. ~keep
#[test]
fn every_clippy_lane_exclusion_is_justified() {
    let unjustified: Vec<&str> = GATE_LANGUAGES
        .iter()
        .filter(|language| language.lanes == Lanes::TextOnly && language.clippy_exclusion_reason.trim().is_empty())
        .map(|language| language.name)
        .collect();
    assert!(
        unjustified.is_empty(),
        "these languages sit out the clippy lane with no stated reason: {unjustified:?}"
    );

    assert!(
        !clippy_lane_languages().is_empty(),
        "no language is in the clippy lane, so the clippy gate would examine nothing"
    );
}
