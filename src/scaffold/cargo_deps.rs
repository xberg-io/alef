//! Cargo dependency-line rendering: the dual-form core-facade dependency, per-target
//! override blocks, cargo-sort-compatible ordering, and extra/workspace dependency merging.

use crate::core::config::{Language, ResolvedCrateConfig};

/// Render a workspace-member core-facade dependency line in DUAL FORM.
///
/// Emits `crate_name = { version = "<version>", path = "<rel_path>"<features> }`.
/// The dual form keeps in-repo dev path builds working (the `path` is always
/// honored when the member crate is present on disk) while letting cargo's
/// package/publish flows (e.g. `maturin sdist`, `cargo package`) strip the
/// `path` and resolve the crate from the registry at `version`.
///
/// `features` is the already-formatted suffix as produced by
/// [`core_dep_features`] — either empty or `, features = ["a", "b"]`. It is
/// appended verbatim so callers control feature selection.
///
/// `version` is the resolved workspace version (the same value used for the
/// generated crate's `[package].version` and by version-sync). The `path` is
/// never altered, so dev builds against the local workspace continue to work.
/// When `version` is empty (no resolvable workspace version, e.g. some unit
/// fixtures), the line falls back to the path-only form so no invalid
/// `version = ""` is emitted.
pub(crate) fn render_core_dep(crate_name: &str, rel_path: &str, features: &str, version: &str) -> String {
    if version.is_empty() {
        format!("{crate_name} = {{ path = \"{rel_path}\"{features} }}")
    } else {
        format!("{crate_name} = {{ version = \"{version}\", path = \"{rel_path}\"{features} }}")
    }
}

/// Like [`render_core_dep`] but honours per-target overrides, mirroring the
/// FFI/Dart backends. Returns `(core_dep_line, target_blocks)`:
///
/// - with no overrides, `core_dep_line` is the single `[dependencies]` line and
///   `target_blocks` is empty (behaviour identical to [`render_core_dep`]);
/// - with overrides, `core_dep_line` is empty and `target_blocks` holds a
///   `[target.'cfg(not(any(<cfg…>)))'.dependencies]` default block plus one
///   `[target.'cfg(<cfg>)'.dependencies]` block per override.
///
/// `default_features` is the pre-formatted feature suffix (e.g. `, features =
/// ["a", "b"]` or `""`), matching [`render_core_dep`]. Callers place
/// `core_dep_line` inside `[dependencies]` when non-empty and append
/// `target_blocks` after that table.
pub(crate) fn render_core_dep_with_overrides(
    crate_name: &str,
    rel_path: &str,
    default_features: &str,
    version: &str,
    overrides: &[crate::core::config::FfiTargetDepOverride],
) -> (String, String) {
    if overrides.is_empty() {
        return (
            render_core_dep(crate_name, rel_path, default_features, version),
            String::new(),
        );
    }

    let combined_cfg = if overrides.len() == 1 {
        overrides[0].cfg.clone()
    } else {
        let cfgs: Vec<String> = overrides.iter().map(|o| o.cfg.clone()).collect();
        format!("any({})", cfgs.join(", "))
    };

    let mut entries: Vec<(String, String)> = vec![(
        format!("not({combined_cfg})"),
        render_core_dep(crate_name, rel_path, default_features, version),
    )];
    for override_ in overrides {
        let default_block = if override_.default_features {
            String::new()
        } else {
            ", default-features = false".to_string()
        };
        let feats = if override_.features.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = override_.features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(", features = [{}]", quoted.join(", "))
        };
        entries.push((
            override_.cfg.clone(),
            render_core_dep(crate_name, rel_path, &format!("{default_block}{feats}"), version),
        ));
    }
    (String::new(), join_sorted_target_dep_blocks(entries))
}

/// Assemble a sequence of `[target.'cfg(...)'.dependencies]` blocks in the
/// table order `cargo-sort` expects: alphabetically by the raw cfg predicate
/// string, using plain byte-wise (case-sensitive) comparison — the same
/// ordering `Vec<String>::sort()` / `str::cmp` produce.
///
/// `entries` is `(cfg_predicate, dependency_line)` pairs — one per target
/// block, including the default `not(...)` branch alongside every override.
/// Emitting the default branch unconditionally first (as earlier revisions of
/// this code did) is only coincidentally correct: `not(...)` sorts after
/// `all(...)` but before `target_os = "..."`, so a config with an `all(...)`
/// override (e.g. the macOS-Intel target) needs its block to precede the
/// default branch. Sorting all entries together — rather than hard-coding the
/// default first — is what keeps `cargo sort --check` passing regardless of
/// which cfg predicates a consumer configures.
///
/// Returns an empty string when `entries` is empty. Each block ends with a
/// trailing newline and blocks are separated by a single blank line, matching
/// the spacing callers already emit between `[dependencies]` and the first
/// target block.
pub(crate) fn join_sorted_target_dep_blocks(mut entries: Vec<(String, String)>) -> String {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(cfg, dep_line)| format!("[target.'cfg({cfg})'.dependencies]\n{dep_line}\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return whether rendered Cargo dependency lines already declare `name`.
///
/// Comparing the complete key before `=` avoids prefix collisions such as
/// `directories-next` being mistaken for `directories`.
pub(crate) fn cargo_dependency_declared<'a>(lines: impl IntoIterator<Item = &'a str>, name: &str) -> bool {
    lines
        .into_iter()
        .any(|line| line.split_once('=').map(|(key, _)| key.trim() == name).unwrap_or(false))
}

/// The sort key `cargo-sort` assigns to one rendered entry of a dependency table: the
/// dependency NAME alone, decoded, with any dotted suffix and any surrounding quotes removed.
///
/// cargo-sort sorts `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` and their
/// `[target.'cfg(...)'.…]` counterparts with `toml_edit`'s `Table::sort_values`, which is
/// `IndexMap::sort_keys` over `Key: Ord`, and `Key::cmp` compares `Key::get()` -- the DECODED
/// text of a single key segment. A dotted entry such as `tracing.workspace = true` parses into
/// one key `tracing` holding a dotted sub-table, so the `.workspace` text is never part of the
/// comparison; a quoted key such as `"tree-sitter" = "1"` is compared unquoted; and a quoted key
/// that itself contains a dot is one segment, not two.
///
/// Sorting the rendered line text instead disagrees with that whenever one dependency name is a
/// prefix of another and the shorter one uses the dotted form: `-` is 0x2D and `.` is 0x2E, so
/// raw text puts `foo-bar = …` before `foo.workspace = true` where cargo-sort puts `foo` first.
/// That is the disagreement that failed `cargo sort --check --workspace` downstream. ~keep
pub(crate) fn dependency_sort_key(line: &str) -> String {
    let mut name = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            name.push(character);
            escaped = false;
            continue;
        }
        match quote {
            Some('"') if character == '\\' => escaped = true,
            Some(open) if character == open => quote = None,
            Some(_) => name.push(character),
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '.' || character == '=' => break,
            None => name.push(character),
        }
    }
    name.trim().to_owned()
}

/// Order rendered dependency-table lines the way `cargo sort --check` requires.
///
/// Stable, and keyed only on [`dependency_sort_key`], exactly matching the stable
/// `IndexMap::sort_keys` cargo-sort performs. Every emitter that writes a `[dependencies]`,
/// `[dev-dependencies]` or `[build-dependencies]` body from a list of rendered lines must sort
/// it through here rather than with `Vec::sort`. ~keep
pub(crate) fn sort_dependency_lines(lines: &mut [String]) {
    lines.sort_by_key(|a| dependency_sort_key(a));
}

///
/// Merges crate-level `extra_dependencies` with per-language overrides via
/// `extra_deps_for_language`, then serializes each entry as a TOML line suitable
/// for appending to a `[dependencies]` section.
///
/// Each value is either:
/// - A string (version only): `cratename = "1.0"`
/// - A TOML table (with path/features/etc.): `cratename = { path = "../foo", features = ["bar"] }`
///
/// Workspace members: when an entry is a path-only table (a `path` key, no
/// `version` key) whose crate name resolves to a workspace member, the resolved
/// workspace version is injected so the table becomes
/// `{ path = "../foo", version = "<v>" }` (dual form). This mirrors
/// [`render_core_dep`] for the core facade and lets cargo-package flows strip
/// the path to a registry version-dependency. `alef.toml` entries stay
/// path-only — the version is injected here at scaffold time. Non-member
/// external deps (e.g. `anyhow = "1.0"`) are emitted unchanged.
///
/// Returns an empty string if no extra dependencies are configured.
pub(crate) fn render_extra_deps(config: &ResolvedCrateConfig, lang: Language) -> String {
    let deps = config.extra_deps_for_language(lang);
    if deps.is_empty() {
        return String::new();
    }
    let member_versions = workspace_member_versions(config);
    let ws_dep_specs = workspace_dep_specs(config);
    let mut lines: Vec<String> = deps
        .iter()
        .map(|(name, value)| match value {
            toml::Value::String(version) => format!("{name} = \"{version}\""),
            toml::Value::Table(table) => {
                if table.get("workspace").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(concrete) = ws_dep_specs.get(name) {
                        return format!("{name} = {concrete}");
                    }
                    return format!("{name} = {value}");
                }
                let needs_version = table.contains_key("path") && !table.contains_key("version");
                if let (true, Some(member_version)) = (needs_version, member_versions.get(name)) {
                    let mut injected = table.clone();
                    injected.insert("version".to_string(), toml::Value::String(member_version.clone()));
                    format!("{name} = {}", toml::Value::Table(injected))
                } else {
                    format!("{name} = {value}")
                }
            }
            other => format!("{name} = {other}"),
        })
        .collect();
    sort_dependency_lines(&mut lines);
    lines.join("\n")
}

/// Resolve the workspace-member crate name → version map for the crate's
/// workspace root.
///
/// Returns an empty map when no workspace root is configured or the root
/// `Cargo.toml` cannot be discovered/parsed — in that case no version is
/// injected and path-only deps are emitted unchanged (matching dev behavior
/// outside a resolvable workspace, e.g. unit tests).
fn workspace_member_versions(config: &ResolvedCrateConfig) -> std::collections::BTreeMap<String, String> {
    let Some(root) = config.workspace_root.as_deref() else {
        return std::collections::BTreeMap::new();
    };
    match crate::publish::workspace::workspace_member_crates(root) {
        Ok(members) => members.versions,
        Err(_) => std::collections::BTreeMap::new(),
    }
}

/// Read the root `Cargo.toml`'s `[workspace.dependencies]` table and return the
/// concrete dependency specs keyed by crate name.
///
/// Used to resolve `{ workspace = true }` extra-dependency entries to concrete
/// specs so out-of-workspace binding crates (e.g. the R package at
/// `packages/r/src/rust/`) compile without a parent workspace. Returns an empty
/// map when no workspace root is configured, the root `Cargo.toml` is absent, or
/// the TOML cannot be parsed.
fn workspace_dep_specs(config: &ResolvedCrateConfig) -> std::collections::BTreeMap<String, toml::Value> {
    let start = config.workspace_root.clone().or_else(|| std::env::current_dir().ok());
    let Some(mut dir) = start else {
        return std::collections::BTreeMap::new();
    };

    if !dir.is_absolute()
        && let Ok(abs) = std::fs::canonicalize(&dir)
    {
        dir = abs;
    }

    loop {
        let cargo_path = dir.join("Cargo.toml");
        if let Ok(contents) = std::fs::read_to_string(&cargo_path)
            && let Ok(doc) = contents.parse::<toml_edit::DocumentMut>()
            && let Some(workspace) = doc.get("workspace")
            && let Some(dependencies) = workspace.get("dependencies")
            && let Some(table) = dependencies.as_table()
        {
            let mut result = std::collections::BTreeMap::new();
            for (key, value) in table.iter() {
                let val_str = value.to_string().trim().to_string();
                let wrapped = format!("x = {}", val_str);
                if let Ok(map) = toml::from_str::<std::collections::HashMap<String, toml::Value>>(&wrapped)
                    && let Some(v) = map.get("x")
                {
                    result.insert(key.to_string(), v.clone());
                }
            }
            return result;
        }
        if !dir.pop() {
            return std::collections::BTreeMap::new();
        }
    }
}

pub(crate) fn render_workspace_dep_or(config: &ResolvedCrateConfig, name: &str, fallback: &str) -> String {
    if workspace_dep_specs(config).contains_key(name) {
        format!("{name}.workspace = true")
    } else {
        format!("{name} = {fallback}")
    }
}
