use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::scaffold::scaffold_meta;
use anyhow::Context as _;
use regex::Regex;
use std::path::{Path, PathBuf};

pub(crate) fn scaffold_wasm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();

    let mut files = vec![];

    let wasm_pkg_name = config.wasm_package_name();

    let core_crate_file = core_crate_dir.replace('-', "_");
    let repository_block = meta
        .configured_repository
        .as_deref()
        .map(|repository| {
            format!(
                r#",
  "repository": {{
    "type": "git",
    "url": "{repository}",
    "directory": "crates/{core_crate_dir}-wasm"
  }}"#
            )
        })
        .unwrap_or_default();
    let license_block = meta
        .license
        .as_deref()
        .map(|license| format!(",\n  \"license\": \"{license}\""))
        .unwrap_or_default();

    // wasm-pack build targets that ship in this package. Each target embeds a ~keep
    // full copy of the wasm binary, so a single-target set keeps the published ~keep
    // package small. Derives `files`, entry points, and the build scripts. ~keep
    let targets = config.wasm_targets();
    const VALID_TARGETS: &[&str] = &["web", "bundler", "nodejs", "deno"];
    if targets.is_empty() {
        anyhow::bail!("[crates.wasm].targets must list at least one wasm-pack target (web, bundler, nodejs, deno)");
    }
    for target in &targets {
        if !VALID_TARGETS.contains(&target.as_str()) {
            anyhow::bail!("[crates.wasm].targets: unknown target '{target}' (valid: web, bundler, nodejs, deno)");
        }
    }
    let has = |t: &str| targets.iter().any(|x| x == t);
    // `main`/`types` prefer the CommonJS-friendly nodejs build; `module` prefers ~keep
    // the browser ES module. When a preferred target isn't built, fall back to ~keep
    // the first configured target. ~keep
    let node_target = if has("nodejs") { "nodejs" } else { targets[0].as_str() };
    let web_target = if has("web") { "web" } else { targets[0].as_str() };
    let exports_block = crate::scaffold::template_env::render(
        "wasm_package_exports.json.jinja",
        crate::alef_context! {
            node_target => node_target,
            web_target => web_target,
            crate_file => core_crate_file,
        },
    );

    // A single-target package publishes just that target's dir; a multi-target ~keep
    // package keeps the broad glob for backward compatibility. ~keep
    let files_block = if targets.len() == 1 {
        format!("[\"pkg/{}\", \"README.md\"]", targets[0])
    } else {
        "[\"pkg\", \"*.wasm\", \"*.d.ts\", \"README.md\"]".to_string()
    };

    let per_target_scripts: String = targets
        .iter()
        .map(|t| format!("    \"build:wasm:{t}\": \"wasm-pack build --release --target {t} --out-dir pkg/{t}\",\n"))
        .collect();
    let build_all = targets
        .iter()
        .map(|t| format!("npm run build:wasm:{t}"))
        .collect::<Vec<_>>()
        .join(" && ");

    let pkg_json = format!(
        r#"{{
  "name": "{wasm_pkg_name}",
  "version": "{version}",
  "private": false,
  "description": "{description}"{license_block}{repository_block},
  "publishConfig": {{
    "access": "public"
  }},
  "type": "module",
  "files": {files_block},
  "main": "pkg/{node_target}/{core_crate_file}_wasm.js",
  "module": "pkg/{web_target}/{core_crate_file}_wasm.js",
  "types": "pkg/{node_target}/{core_crate_file}_wasm.d.ts",
  {exports_block}  "engines": {{
    "node": "{node_engine}"
  }},
  "scripts": {{
    "build": "wasm-pack build --target {node_target} --out-dir pkg/{node_target}",
    "build:ci": "wasm-pack build --release --target {node_target} --out-dir pkg/{node_target}",
{per_target_scripts}    "build:all": "{build_all} && find pkg -name .gitignore -delete",
    "test": "vitest run",
    "test:watch": "vitest watch",
    "test:coverage": "vitest run --coverage",
    "clean": "rm -rf pkg dist"
  }},
  "devDependencies": {{
    "vitest": "{vitest}",
    "@vitest/coverage-v8": "{vitest_coverage_v8}"
  }}
}}
"#,
        wasm_pkg_name = wasm_pkg_name,
        version = version,
        description = meta.description,
        license_block = license_block,
        repository_block = repository_block,
        files_block = files_block,
        node_target = node_target,
        web_target = web_target,
        core_crate_file = core_crate_file,
        exports_block = exports_block,
        node_engine = tv::npm::NODE_ENGINE,
        per_target_scripts = per_target_scripts,
        build_all = build_all,
        // `test`/`test:watch`/`test:coverage` above invoke `vitest` directly (not `pnpm exec`
        // or `npx`), so this package must declare it as a devDependency or a frozen install
        // resolves nothing for those scripts to run -- same central version the e2e wasm/
        // typescript generators pin (`tv::npm::VITEST`), so this package and its e2e sibling
        // can never drift apart on which vitest they install. `test:coverage` additionally
        // needs its coverage provider declared, or `vitest run --coverage` fails to load a
        // reporter -- `tv::npm::VITEST_COVERAGE_V8` tracks vitest's own version (see that
        // const's doc). ~keep
        vitest = tv::npm::VITEST,
        vitest_coverage_v8 = tv::npm::VITEST_COVERAGE_V8,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}

/// Crate-visible entry point for every in-place repair this module makes to a pre-existing
/// `crates/<crate>-wasm/package.json` -- the single call site `cli::pipeline::generate::scaffold`
/// invokes, so a future repair joins this list once instead of growing the call site again.
///
/// `crates/*-wasm/package.json` is `generated_header: false` (create-only: see
/// `write_scaffold_files_report`'s ownership guard in `cli::pipeline::generate::scaffold`), so
/// `scaffold_wasm` never rewrites it once it exists on disk -- every defect a later `scaffold_wasm`
/// fix closes keeps shipping in every crate scaffolded before that fix, forever, unless something
/// repairs the file in place. Each repair below is independently safe to run in isolation (its own
/// recognition anchor, its own idempotency) and independently safe to run in this fixed order:
/// [`migrate_wasm_package_json_exports`] first, then
/// [`migrate_wasm_package_json_vitest_dev_dependencies`] reading whatever the first repair just
/// wrote, so neither ever acts on a stale in-memory copy of the other's edit. Returns `true` when
/// either repair changed the file on disk.
pub(crate) fn migrate_wasm_package_json(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let exports_changed = migrate_wasm_package_json_exports(base_dir, relative_path)?;
    let vitest_changed = migrate_wasm_package_json_vitest_dev_dependencies(base_dir, relative_path)?;
    Ok(exports_changed || vitest_changed)
}

/// Repair a pre-existing `crates/<crate>-wasm/package.json` that predates the `exports` map
/// [`scaffold_wasm`] now emits (added in the fix that also introduced
/// `wasm_package_exports.json.jinja`).
///
/// `crates/*-wasm/package.json` is `generated_header: false` (create-only: see
/// `write_scaffold_files_report`'s ownership guard in `cli::pipeline::generate::scaffold`), so a
/// repo scaffolded before the `exports` map existed keeps shipping a `package.json` with no
/// `exports` key forever — `require()`/`import` resolution under Node's package-exports
/// enforcement then falls back to legacy `main`/`module` resolution, which still works for the
/// package root but leaves any consumer relying on subpath/conditional exports (`browser`,
/// dual CJS/ESM `require`+`import`) unresolvable. A full regenerate-and-overwrite is not
/// attempted: `package.json` is exactly the kind of file consumers hand-edit (extra
/// `devDependencies`, custom `scripts`), so this only ever *inserts* the missing block, never
/// touches anything else on the line, and refuses outright rather than guess when the file
/// doesn't unambiguously carry alef's own `main`/`module`/`types` shape. See
/// [`repair_missing_wasm_exports`] for the exact detection and insertion. Called only from
/// [`migrate_wasm_package_json`], which is the crate-visible entry point for every repair this
/// module makes to a pre-existing `crates/*-wasm/package.json`. ~keep
fn migrate_wasm_package_json_exports(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Some(migrated) = repair_missing_wasm_exports(&existing) else {
        return Ok(false);
    };
    if migrated == existing {
        return Ok(false);
    }

    let parent = path
        .parent()
        .context("wasm package.json path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing crates/*-wasm/package.json: inserted the missing \"exports\" map"
    );
    Ok(true)
}

/// The `(node_target, crate_file, web_target)` triple recovered from a `crates/*-wasm/package.json`'s
/// own `"main"`/`"module"`/`"types"` fields, when and only when all three agree on the same
/// `crate_file` and follow the exact `pkg/<target>/<crate_file>_wasm.js` / `.d.ts` naming
/// convention `scaffold_wasm` always emits.
///
/// This is the single recognition fingerprint every migration in this module anchors on before
/// touching a `crates/*-wasm/package.json` it did not just write. The three-field cross-agreement
/// is specific enough that no foreign, hand-authored `package.json` plausibly matches by
/// coincidence: an ordinary manifest's `main`/`module`/`types` point at unrelated file names (or
/// omit one of the three fields entirely), which fails at least one capture or the `crate_file` /
/// `node_target` equality checks below and returns `None`. Sharing one function keeps every call
/// site answering "is this alef's own file" from identical evidence, rather than two migrations
/// silently drifting to slightly different fingerprints over time. A caller that finds `Some(..)`
/// still is not free to rewrite blindly — each migration adds its own further anchor (an exact
/// `"engines": {` line, an exact `"test": "vitest run"` script) proving the *specific* region it
/// is about to touch still matches its own template's output, not just that the file is alef's in
/// general. ~keep
fn recognize_alef_wasm_package_json(content: &str) -> Option<(&str, &str, &str)> {
    let main_pattern = Regex::new(r#""main":\s*"pkg/([^/"]+)/([^"]+)_wasm\.js""#).expect("valid regex");
    let module_pattern = Regex::new(r#""module":\s*"pkg/([^/"]+)/([^"]+)_wasm\.js""#).expect("valid regex");
    let types_pattern = Regex::new(r#""types":\s*"pkg/([^/"]+)/([^"]+)_wasm\.d\.ts""#).expect("valid regex");

    let main_captures = main_pattern.captures(content)?;
    let module_captures = module_pattern.captures(content)?;
    let types_captures = types_pattern.captures(content)?;

    let node_target = main_captures.get(1)?.as_str();
    let crate_file = main_captures.get(2)?.as_str();
    let web_target = module_captures.get(1)?.as_str();
    if module_captures.get(2)?.as_str() != crate_file {
        return None;
    }
    if types_captures.get(1)?.as_str() != node_target || types_captures.get(2)?.as_str() != crate_file {
        return None;
    }
    Some((node_target, crate_file, web_target))
}

/// Pure text transform behind [`migrate_wasm_package_json_exports`]. Returns `None` when
/// `content` is not a safe migration candidate at all: it already has an `"exports"` key
/// (nothing missing — whether that's this fix's own output or a consumer's hand-added one, either
/// way there is nothing to insert without risking a duplicate or clobbering a custom map),
/// [`recognize_alef_wasm_package_json`] cannot positively identify it as alef's own wasm
/// package.json, or the file has no `"engines": {` line to anchor the insertion before (the one
/// point in the template `scaffold_wasm` always emits the block directly ahead of).
///
/// `node_target`/`web_target`/`crate_file` for the newly rendered block are extracted from the
/// file's own `main`/`module`/`types` fields, not recomputed from live config — the values already
/// on disk are definitionally consistent with the rest of the file, so reusing them (rather than
/// asking the current `ResolvedCrateConfig` what it thinks the targets are today) can never
/// disagree with the paths those existing `main`/`module`/`types` lines already point at.
fn repair_missing_wasm_exports(content: &str) -> Option<String> {
    if content.contains("\"exports\":") {
        return None;
    }

    let (node_target, crate_file, web_target) = recognize_alef_wasm_package_json(content)?;

    let engines_line_index = content.lines().position(|line| line.trim() == "\"engines\": {")?;

    let exports_block = crate::scaffold::template_env::render(
        "wasm_package_exports.json.jinja",
        crate::alef_context! {
            node_target => node_target,
            web_target => web_target,
            crate_file => crate_file,
        },
    );
    // The template's first line carries no leading indent of its own -- `scaffold_wasm` supplies
    // it via the two literal spaces ahead of `{exports_block}` in its format string -- so that
    // indent has to be added back here for the block to line up once spliced in as whole lines.
    // Every later line already bakes in its own absolute indent (matching that same 2-space
    // base), so only the first line needs it. ~keep
    let mut export_lines: Vec<String> = exports_block.lines().map(str::to_string).collect();
    if let Some(first) = export_lines.first_mut() {
        *first = format!("  {first}");
    }

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    lines.splice(engines_line_index..engines_line_index, export_lines);
    let mut joined = lines.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// Repair a pre-existing `crates/<crate>-wasm/package.json` whose `scripts.test` /
/// `test:watch` / `test:coverage` invoke `vitest` (and, for `test:coverage`, its coverage
/// provider) with no matching `devDependencies` entry to back them -- the defect the sibling fix
/// closed for freshly scaffolded crates. `generated_header: false` means `scaffold_wasm` never
/// rewrites this file once it exists on disk, so every crate scaffolded before that fix (or
/// scaffolded after it but before the coverage provider was added) keeps shipping a manifest
/// whose own test scripts have nothing installed to run under a frozen lockfile, forever, unless
/// something repairs it in place. See [`repair_missing_wasm_vitest_dev_dependencies`] for the
/// exact detection and insertion, and [`repair_missing_wasm_exports`]'s doc comment for why an
/// in-place text patch -- never a full regenerate-and-overwrite -- is the only safe shape for a
/// `generated_header: false` file a consumer may have hand-edited. Called only from
/// [`migrate_wasm_package_json`].
fn migrate_wasm_package_json_vitest_dev_dependencies(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = base_dir.join(relative_path);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Some(migrated) = repair_missing_wasm_vitest_dev_dependencies(&existing) else {
        return Ok(false);
    };
    if migrated == existing {
        return Ok(false);
    }

    let parent = path
        .parent()
        .context("wasm package.json path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing crates/*-wasm/package.json: inserted the missing vitest devDependency"
    );
    Ok(true)
}

/// Pure text transform behind [`migrate_wasm_package_json_vitest_dev_dependencies`]. Returns
/// `None` when `content` is not a safe migration candidate: [`recognize_alef_wasm_package_json`]
/// cannot positively identify it as alef's own wasm package.json, it does not carry the literal
/// `"test": "vitest run"` script `scaffold_wasm` always emits (a second, independent anchor --
/// the main/module/types fingerprint alone proves the file's *build* shape, not that its
/// `scripts` block still matches what this migration patches), or every dependency this
/// migration would add is already declared (nothing left to do -- this is what makes a second
/// run a no-op).
///
/// Two on-disk shapes are handled, and each is recognized by its own exact anchor rather than
/// guessed at:
/// - no `"devDependencies"` key at all (the shape every crate scaffolded before the vitest fix
///   shipped in): a whole new block is inserted via
///   [`insert_new_dev_dependencies_block`], anchored on the literal two-line `  }` / `}` suffix
///   `scaffold_wasm` emits when `scripts` is its last key. A consumer who added a field of their
///   own after `scripts` breaks that exact suffix and is left untouched rather than guessed at.
/// - a `"devDependencies"` key already present (freshly scaffolded after the vitest fix but
///   before the coverage provider was added, or a consumer who added unrelated dev dependencies
///   of their own by hand): [`insert_into_existing_dev_dependencies`] inserts only what's
///   missing, as new lines directly after the `"devDependencies": {` line, never touching
///   anything already inside the object.
fn repair_missing_wasm_vitest_dev_dependencies(content: &str) -> Option<String> {
    recognize_alef_wasm_package_json(content)?;
    if !content.contains(r#""test": "vitest run""#) {
        return None;
    }
    let needs_coverage_provider = content.contains(r#""test:coverage":"#);

    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    match parsed.get("devDependencies") {
        None => insert_new_dev_dependencies_block(content, needs_coverage_provider),
        Some(existing) => {
            let existing = existing.as_object()?;
            let needs_vitest = !existing.contains_key("vitest");
            let needs_coverage = needs_coverage_provider && !existing.contains_key("@vitest/coverage-v8");
            if !needs_vitest && !needs_coverage {
                return None;
            }
            insert_into_existing_dev_dependencies(content, needs_vitest, needs_coverage)
        }
    }
}

/// Splice a fresh `"devDependencies"` block into a `crates/*-wasm/package.json` that has none at
/// all, anchored on the literal two-line `  }` / `}` suffix `scaffold_wasm` emits whenever
/// `scripts` is its last key -- the same exact-suffix caution [`repair_missing_wasm_exports`]
/// applies to its `"engines": {` anchor, applied here to the tail of the file instead of a fixed
/// line. Returns `None` when that exact suffix is not found, which is what keeps a consumer's own
/// trailing field (added after `scripts` by hand) from ever earning a guessed insertion point.
fn insert_new_dev_dependencies_block(content: &str, include_coverage_provider: bool) -> Option<String> {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let last = lines.len().checked_sub(1)?;
    let scripts_close = last.checked_sub(1)?;
    if lines[last] != "}" || lines[scripts_close] != "  }" {
        return None;
    }

    let mut entries = vec![format!("\"vitest\": \"{}\"", tv::npm::VITEST)];
    if include_coverage_provider {
        entries.push(format!("\"@vitest/coverage-v8\": \"{}\"", tv::npm::VITEST_COVERAGE_V8));
    }
    let entry_count = entries.len();

    lines[scripts_close] = "  },".to_string();
    let mut block = vec!["  \"devDependencies\": {".to_string()];
    for (index, entry) in entries.into_iter().enumerate() {
        let suffix = if index + 1 == entry_count { "" } else { "," };
        block.push(format!("    {entry}{suffix}"));
    }
    block.push("  }".to_string());

    lines.splice(last..last, block);
    let mut joined = lines.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// Insert missing entries as new lines directly after an existing `"devDependencies": {` line,
/// never touching anything already inside the object -- no closing-brace search, no
/// reformatting of neighboring entries, so a consumer's own hand-added dependencies (any name,
/// any formatting, including a multi-line detailed table) survive untouched regardless of shape.
/// Returns `None` when the literal `  "devDependencies": {` opening line (the exact indent
/// `scaffold_wasm` always emits at the top level) is not found, the same "positively recognized
/// shape or refuse" rule every insertion in this module follows.
///
/// Checks whether the object was already empty (its very next line closes it) so the last
/// inserted entry omits its trailing comma exactly when nothing follows it before the closing
/// brace -- otherwise every inserted line is comma-terminated, since something (an existing entry
/// or the closing brace's own object) always follows. ~keep
fn insert_into_existing_dev_dependencies(content: &str, needs_vitest: bool, needs_coverage: bool) -> Option<String> {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let open_index = lines.iter().position(|line| line == "  \"devDependencies\": {")?;
    let object_is_empty = lines
        .get(open_index + 1)
        .is_some_and(|line| matches!(line.trim(), "}" | "},"));

    let mut entries = Vec::new();
    if needs_vitest {
        entries.push(format!("\"vitest\": \"{}\"", tv::npm::VITEST));
    }
    if needs_coverage {
        entries.push(format!("\"@vitest/coverage-v8\": \"{}\"", tv::npm::VITEST_COVERAGE_V8));
    }
    let entry_count = entries.len();

    let new_lines: Vec<String> = entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let omit_comma = object_is_empty && index + 1 == entry_count;
            let suffix = if omit_comma { "" } else { "," };
            format!("    {entry}{suffix}")
        })
        .collect();

    lines.splice(open_index + 1..open_index + 1, new_lines);
    let mut joined = lines.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// The wasm-only `.cargo/config.toml` seed, emitted when no `[scaffold.cargo]` is configured.
///
/// Split out of `scaffold` so it is reachable from a test: the emit site is gated on
/// `Path::new(".cargo/config.toml").exists()` against the **process CWD**, not the write
/// `base_dir`, so a test that goes through `scaffold` observes whatever the repo running the test
/// happens to have at its root — which is how a marker regression on this file stays invisible.
/// See `rust_toolchain_file` for the same reasoning on the other create-once seed. ~keep
pub(crate) fn wasm_cargo_config_file() -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(".cargo/config.toml"),
        content: "[build]\nincremental = true\n\n[target.wasm32-unknown-unknown]\nrustflags = [\"-C\", \"target-feature=+bulk-memory\", \"--cfg\", \"getrandom_backend=\\\"wasm_js\\\"\", \"-C\", \"link-arg=--allow-multiple-definition\"]\n\n[net]\ngit-fetch-with-cli = true\n\n[registries.crates-io]\nprotocol = \"sparse\"\n".to_string(),
        // The `[scaffold.cargo]` branch above writes the *same path* through
        // `render_cargo_config`, which hand-rolls its own "auto-generated by alef" header and so
        // is claimed, stamped and skipped by poly. This branch emitted the same path unmarked, so
        // which of the two a repo got decided whether `.cargo/config.toml` carried an
        // `alef:hash:` line at all — and an unmarked-but-alef-authored file is exactly the state
        // poly reformats and `alef verify` cannot see. Both branches now land on the marker rail.
        // Safe despite the ownership guard's create-once trap because the emit is already gated
        // on the file not existing, so the write is always a create. ~keep
        generated_header: true,
    }
}

/// The exact `.cargo/config.toml` body `scaffold` emitted for wasm-only projects (no
/// `[scaffold.cargo]` configured) before the fix that added
/// `-C link-arg=--allow-multiple-definition` to the wasm32 rustflags.
pub(crate) const STALE_WASM_CARGO_CONFIG: &str = "[build]\nincremental = true\n\n[target.wasm32-unknown-unknown]\nrustflags = [\"-C\", \"target-feature=+bulk-memory\", \"--cfg\", \"getrandom_backend=\\\"wasm_js\\\"\"]\n\n[net]\ngit-fetch-with-cli = true\n\n[registries.crates-io]\nprotocol = \"sparse\"\n";

/// Repair a pre-existing `.cargo/config.toml` that still carries the pre-fix wasm32 rustflags
/// -- the exact defect fixed when this file's hardcoded, wasm-only literal above gained
/// `-C link-arg=--allow-multiple-definition` (`cda088792`, "allow multiple definition on
/// wasm32 link"). wasm32-unknown-unknown has no unified libc, so multiple C dependencies
/// (tree-sitter's wasm shim, a WASI-built Tesseract) can each ship functionally-equivalent
/// libc stubs that `wasm-ld` rejects as duplicate definitions without this flag; a repo that
/// never happens to combine such dependencies never hits the failure, which is why this can
/// stay unnoticed indefinitely once scaffolded.
///
/// This file is unusual among the create-once scaffold seeds above: `scaffold()`'s `else if`
/// arm only ever pushes it into the returned `files` list when `.cargo/config.toml` does
/// *not already exist* on disk, so once a repo has one it drops out of `files` entirely and
/// never reaches `write_scaffold_files_report`'s per-file ownership guard the way the other
/// create-once seeds (the zig/dart/swift placeholders, `.pubignore`, `example.zig`) do.
/// Detection here is therefore unconditional on the generated file list and purely
/// content-driven: an exact byte match against the one known-bad constant. This file carries
/// no per-project variables at all (this branch only fires without `[scaffold.cargo]`
/// configured, so nothing here is templated), so exact-match is both sufficient and maximally
/// conservative -- any consumer edit at all leaves the file completely untouched. ~keep
pub(crate) fn migrate_wasm_cargo_config_allow_multiple_definition(base_dir: &std::path::Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(
        base_dir,
        std::path::Path::new(".cargo/config.toml"),
    )?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if existing != STALE_WASM_CARGO_CONFIG {
        return Ok(false);
    }

    // Taken from the emitter rather than repeated as a fourth copy of the same literal: a repair
    // that writes bytes the emitter no longer produces converges the file onto a body no
    // subsequent run agrees with. The header is deliberately absent — `write_scaffold_files_report`
    // adds it, and this path writes directly, so a header written here would be claimed by
    // `content_has_alef_marker` and never stamped, which is the poly ping-pong state. ~keep
    let replacement = wasm_cargo_config_file().content;

    let parent = path
        .parent()
        .context(".cargo/config.toml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, replacement.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing .cargo/config.toml: added -C link-arg=--allow-multiple-definition \
         to the wasm32-unknown-unknown rustflags"
    );
    Ok(true)
}

#[cfg(test)]
mod migrate_tests;
