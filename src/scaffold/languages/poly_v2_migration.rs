//! In-place conversion of a consumer's v1 `poly.toml` to the v2 schema poly 0.36 reads.
//!
//! [`crate::scaffold::scaffold_poly_config`] emits v2 tables (`[engines.*]`, `[per_file_ignores]`,
//! `[workspace] run`), but `merge_managed_toml`'s union pass only ever adds tables, so an
//! already-scaffolded consumer would keep its v1 `[lint.*]`, `[fmt.*]` and `[per-file-ignores]`
//! tables beside the new ones. poly rejects a file that mixes the two schemas, so the merge alone
//! would turn every consumer's lint red. This pass rewrites the v1 tables the way
//! `poly migrate --to-v2` does, keeping each table's comments, before the merge runs.

use anyhow::Context as _;
use std::collections::BTreeMap;
use std::path::Path;

/// Path of the repo-root poly config this migration rewrites, relative to the repo root.
const POLY_CONFIG_RELATIVE: &str = "poly.toml";

/// v1 `[lint.<name>]` tables that configure a cross-cutting engine rather than a language.
const CROSS_CUTTING_ENGINES: &[&str] = &["typos", "astgrep", "uncomment", "quality"];

/// Engines that serve exactly one language: their v1 `[lint.<lang>.<tool>]` table becomes
/// `[engines.<tool>.lint]`, as `poly migrate --to-v2` writes it, instead of a `lang` sub-table.
const SINGLE_LANGUAGE_ENGINES: &[(&str, &str)] = &[("ruff", "python"), ("mago", "php"), ("shfmt", "shell")];

/// Rewrite a pre-existing v1 `poly.toml` to the v2 schema.
///
/// Silent (returns `Ok(false)`) on a missing `poly.toml`, unparsable TOML, or a file with no v1
/// tables left (idempotent). Unknown keys directly under `[lint]` or `[fmt]` stay where they are,
/// so poly reports the mixed file instead of alef dropping configuration it cannot place.
pub(crate) fn migrate_poly_toml_to_v2(base_dir: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, POLY_CONFIG_RELATIVE.as_ref())?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut doc) = existing.parse::<toml_edit::DocumentMut>() else {
        return Ok(false);
    };
    if !convert_v1_tables(doc.as_table_mut()) {
        return Ok(false);
    }

    let parent = path.parent().context("poly.toml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, doc.to_string().as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    tracing::info!(path = %path.display(), "converted pre-existing poly.toml to the v2 schema");
    Ok(true)
}

/// Convert every v1 construct in `root`; returns whether anything changed.
pub(crate) fn convert_v1_tables(root: &mut toml_edit::Table) -> bool {
    let mut changed = false;
    if let Some(item) = root.remove("per-file-ignores") {
        merge_into(table_at(root, &["per_file_ignores"]), item);
        changed = true;
    }
    for (v1_key, stage) in [("lint", "lint"), ("fmt", "format")] {
        if let Some(toml_edit::Item::Table(table)) = root.remove(v1_key) {
            let leftover = convert_stage(root, table, stage);
            if !leftover.is_empty() {
                root.insert(v1_key, toml_edit::Item::Table(leftover));
            }
            changed = true;
        }
    }
    if let Some(toml_edit::Item::Table(rules)) = root.remove("rules") {
        let astgrep = table_at(root, &["engines", "astgrep"]);
        for (key, item) in rules {
            let key = match key.as_str() {
                "dirs" => "rule_dirs".to_owned(),
                "builtin" => "builtin_pack".to_owned(),
                _ => key,
            };
            astgrep.insert(&key, item);
        }
        changed = true;
    }
    if let Some(workspace_root) = root
        .get_mut("workspace")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|workspace| workspace.remove("root"))
    {
        root.insert("root", workspace_root);
        changed = true;
    }
    if changed {
        // v1 ran typos unless a config switched it off; v2 runs it only when enabled. ~keep
        let typos = table_at(root, &["engines", "typos"]);
        if !typos.contains_key("enabled") {
            typos.insert("enabled", toml_edit::value(true));
        }
    }
    changed
}

/// Move the tables of one v1 stage (`[lint]` or `[fmt]`) under `[engines]`, returning the keys
/// that have no v2 home.
fn convert_stage(root: &mut toml_edit::Table, stage_table: toml_edit::Table, stage: &str) -> toml_edit::Table {
    let mut leftover = toml_edit::Table::new();
    leftover.set_implicit(true);
    for (key, item) in stage_table {
        if stage == "lint"
            && key == "workspace"
            && let Some(run) = item.as_bool()
        {
            let run = if run { "auto" } else { "never" };
            table_at(root, &["workspace"]).insert("run", toml_edit::value(run));
            continue;
        }
        let toml_edit::Item::Table(table) = item else {
            leftover.insert(&key, item);
            continue;
        };
        if CROSS_CUTTING_ENGINES.contains(&key.as_str()) {
            let path: Vec<&str> = if stage == "lint" {
                vec!["engines", &key]
            } else {
                vec!["engines", &key, stage]
            };
            merge_into(table_at(root, &path), toml_edit::Item::Table(table));
            continue;
        }
        let language = key;
        let mut language_leftover = toml_edit::Table::new();
        language_leftover.set_implicit(true);
        for (tool, tool_item) in table {
            if !tool_item.is_table() {
                language_leftover.insert(&tool, tool_item);
                continue;
            }
            let single = SINGLE_LANGUAGE_ENGINES
                .iter()
                .any(|(engine, lang)| *engine == tool && *lang == language);
            let path: Vec<&str> = if single {
                vec!["engines", &tool, stage]
            } else {
                vec!["engines", &tool, "lang", &language, stage]
            };
            merge_into(table_at(root, &path), tool_item);
        }
        if !language_leftover.is_empty() {
            leftover.insert(&language, toml_edit::Item::Table(language_leftover));
        }
    }
    leftover
}

/// The table at `path` under `root`, creating implicit (header-less) parents on the way.
fn table_at<'a>(root: &'a mut toml_edit::Table, path: &[&str]) -> &'a mut toml_edit::Table {
    let mut table = root;
    for segment in path {
        let entry = table.entry(segment).or_insert_with(|| {
            let mut child = toml_edit::Table::new();
            child.set_implicit(true);
            toml_edit::Item::Table(child)
        });
        if !entry.is_table() {
            let mut child = toml_edit::Table::new();
            child.set_implicit(true);
            *entry = toml_edit::Item::Table(child);
        }
        table = entry.as_table_mut().expect("entry was just made a table");
    }
    table
}

/// Merge a moved v1 table (or inline table) into its v2 destination; keys already present at the
/// destination win, since a consumer who wrote both spellings meant the v2 one.
fn merge_into(destination: &mut toml_edit::Table, source: toml_edit::Item) {
    match source {
        toml_edit::Item::Table(source) => {
            if destination.is_empty() {
                let implicit = source.is_implicit();
                *destination = source;
                destination.set_implicit(implicit && destination.iter().all(|(_, item)| item.is_table()));
                return;
            }
            for (key, item) in source {
                if !destination.contains_key(&key) {
                    destination.insert(&key, item);
                }
            }
        }
        toml_edit::Item::Value(toml_edit::Value::InlineTable(inline)) => {
            for (key, value) in inline {
                if !destination.contains_key(&key) {
                    destination.insert(&key, toml_edit::Item::Value(value));
                }
            }
        }
        _ => {}
    }
}

/// v1 table prefixes alef itself emitted, and where the v2 schema keeps the same arrays.
const V1_PROVENANCE_PREFIXES: &[(&str, &str)] = &[
    ("per-file-ignores.", "per_file_ignores."),
    ("lint.markdown.rumdl.", "engines.rumdl.lang.markdown.lint."),
    ("fmt.markdown.rumdl.", "engines.rumdl.lang.markdown.format."),
    ("lint.python.ruff.", "engines.ruff.lint."),
    ("lint.php.mago.", "engines.mago.lint."),
];

/// Convert an existing `poly.toml` about to be merged with alef's v2 output, and respell the
/// merge-provenance paths recorded against its v1 tables. Runs on the read-only preview path too,
/// so `alef verify` compares against the file a regeneration would write.
pub(crate) fn convert_v1_merge(
    existing: &mut toml_edit::Table,
    previous_generated_arrays: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    convert_v1_tables(existing);
    previous_generated_arrays
        .iter()
        .map(|(path, values)| (v2_provenance_path(path), values.clone()))
        .collect()
}

/// The v2 spelling of a merge-provenance array path recorded against a v1 `poly.toml`, so the
/// first merge after the conversion still prunes values alef stopped proposing.
pub(crate) fn v2_provenance_path(path: &str) -> String {
    V1_PROVENANCE_PREFIXES
        .iter()
        .find_map(|(v1, v2)| path.strip_prefix(v1).map(|rest| format!("{v2}{rest}")))
        .unwrap_or_else(|| path.to_owned())
}
