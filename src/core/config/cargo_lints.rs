//! Raw `[lints.rust]` / `[lints.clippy]` passthrough for generated binding-crate
//! `Cargo.toml` manifests.
//!
//! Generated binding crates (`*-ffi`, `*-node`, `*-py`, `*-php`, ...) are full
//! Cargo packages that only opt into the workspace's deny-by-default lint policy
//! (`print_stdout`, `dbg_macro`, `unused_must_use`, `unexpected_cfgs` check-cfg
//! allowlists, ...) by carrying their own `[lints.rust]` / `[lints.clippy]` table —
//! `[lints]\nworkspace = true` is not an option here, since it is all-or-nothing and
//! would drag in `unsafe_code = "deny"`, which an FFI crate cannot satisfy. Before
//! this module, `alef.toml` had no key that could express such a table, so
//! consumers hand-added it directly to the generated manifest; the next `alef
//! generate` silently overwrote it back out, going unnoticed because Cargo just
//! stops enforcing the denies rather than erroring.
//!
//! [`CargoLintsConfig`] gives that table a declarative home. Beyond the passthrough,
//! [`BUILTIN_CLIPPY_DEFAULTS`] makes the three-lint `[lints.clippy]` deny block itself a
//! built-in: every generated binding crate gets it whether or not `[crates.cargo_lints]` is
//! configured at all, since making it opt-in reproduced the exact bug this module was built
//! to fix -- a consumer who never wrote the config key (or who, as this module's own
//! changelog entry above records, hand-added the block directly instead) still silently lost
//! enforcement on the next `alef generate`. A configured entry for one of the three keys still
//! wins verbatim over the built-in, so a crate with a real reason to relax one can. Everything
//! else about the table remains a pure passthrough — alef does not validate or interpret lint
//! names, mirroring [`super::manifest_extras::ManifestExtras`] for the
//! language-native-manifest equivalent. ~keep

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Built-in `[lints.clippy]` entries every alef-generated binding crate carries
/// unconditionally, merged underneath any consumer-configured `clippy` entries so a
/// same-named consumer entry still wins. These three mirror the xberg-io family's
/// "tracing is the only logging surface" convention, which requires each generated crate to
/// opt into the denies itself since `[lints]\nworkspace = true` is not available to it (see
/// the module doc). Sorted by key: [`CargoLintsConfig::render`] and
/// [`CargoLintsConfig::clippy_block`] both render a `BTreeMap`, so this list's own order is
/// cosmetic, but keeping it sorted here avoids the constant reading as if insertion order
/// mattered. ~keep
const BUILTIN_CLIPPY_DEFAULTS: &[(&str, &str)] = &[
    ("dbg_macro", "deny"),
    ("print_stderr", "deny"),
    ("print_stdout", "deny"),
];

/// The `[lints.cargo]` table every generated binding crate carries. Its manifests list
/// dependencies the generated code only sometimes uses (`async-trait`, `tracing`, ...) and
/// guard them for `cargo machete` through `[package.metadata.cargo-machete] ignored`. Cargo's
/// own `unused_dependencies` lint (Rust 1.100) does not read that key and, with
/// `build.warnings = "deny"`, fails the build, so it is allowed here. ~keep
const CARGO_LINTS_BLOCK: &str = "[lints.cargo]\nunused_dependencies = \"allow\"";

/// Raw `[lints.rust]` / `[lints.clippy]` tables for a generated binding-crate
/// `Cargo.toml`. Each entry's value may be a bare string (`print_stdout = "deny"`)
/// or a table (`unexpected_cfgs = { level = "warn", check-cfg = [...] }`) — alef
/// never inspects the value, only splices it in verbatim.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CargoLintsConfig {
    /// Entries for the generated crate's `[lints.rust]` table.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub rust: BTreeMap<String, toml::Value>,
    /// Entries for the generated crate's `[lints.clippy]` table.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub clippy: BTreeMap<String, toml::Value>,
}

impl CargoLintsConfig {
    /// True when neither table has any *configured* entries. This reflects only what the
    /// consumer wrote in `[crates.cargo_lints]` — it does not account for
    /// [`BUILTIN_CLIPPY_DEFAULTS`], which [`Self::render`] and [`Self::clippy_block`] always
    /// merge in regardless of this value.
    pub fn is_empty(&self) -> bool {
        self.rust.is_empty() && self.clippy.is_empty()
    }

    /// The `clippy` table merged with [`BUILTIN_CLIPPY_DEFAULTS`] — a configured entry wins
    /// over the built-in default for the same key.
    fn effective_clippy(&self) -> BTreeMap<String, toml::Value> {
        let mut merged: BTreeMap<String, toml::Value> = BUILTIN_CLIPPY_DEFAULTS
            .iter()
            .map(|(key, value)| ((*key).to_string(), toml::Value::String((*value).to_string())))
            .collect();
        merged.extend(self.clippy.iter().map(|(key, value)| (key.clone(), value.clone())));
        merged
    }

    /// Render `[lints.rust]` / `[lints.clippy]` tables for splicing into a generated
    /// Cargo.toml. `[lints.rust]` is omitted when no `rust` entries are configured;
    /// `[lints.clippy]` always renders, since [`Self::effective_clippy`] is never empty.
    /// The returned text carries no leading or trailing newline — callers own the
    /// blank-line glue that fits their own template.
    pub fn render(&self) -> String {
        render_lint_tables(&self.rust, &self.effective_clippy())
    }

    /// The `[lints.cargo]` table alone, for backends that hand-assemble their lint tables.
    pub fn cargo_block(&self) -> &'static str {
        CARGO_LINTS_BLOCK
    }

    /// `"key = value"` lines for every `[lints.rust]` entry except `exclude`, sorted
    /// by key. Used by backends (dart, swift, elixir) that already emit their own
    /// `unexpected_cfgs` check-cfg allowlist into `[lints.rust]` as a hand-written
    /// literal: Cargo allows only one `[lints.rust]` table per manifest, so a
    /// configured entry has to become an extra sibling line under that same
    /// hand-written header rather than open a second one. Excluding the builtin
    /// key here (rather than overwriting it after merging through `toml::Value`)
    /// keeps the builtin's own literal formatting byte-for-byte stable when no
    /// passthrough entry collides with it — this repo's Cargo.toml snapshot tests
    /// pin that exact text. A same-named user entry is dropped: the builtin encodes
    /// a compile-correctness requirement (the allowlist matching the crate's actual
    /// cfg gates) a raw passthrough config has no way to know about. ~keep
    pub fn extra_rust_lines(&self, exclude: &[&str]) -> Vec<String> {
        self.rust
            .iter()
            .filter(|(key, _)| !exclude.contains(&key.as_str()))
            .map(|(key, value)| format!("{key} = {value}"))
            .collect()
    }

    /// The `[lints.clippy]` table alone, rendered the same way [`Self::render`]
    /// would. Never empty — [`BUILTIN_CLIPPY_DEFAULTS`] guarantees at least those three
    /// entries even when `clippy` is unconfigured. Pairs with [`Self::extra_rust_lines`]
    /// for backends that hand-assemble `[lints.rust]`.
    pub fn clippy_block(&self) -> String {
        render_table("[lints.clippy]", &self.effective_clippy())
    }
}

fn render_lint_tables(rust: &BTreeMap<String, toml::Value>, clippy: &BTreeMap<String, toml::Value>) -> String {
    let mut sections = Vec::new();
    if !rust.is_empty() {
        sections.push(render_table("[lints.rust]", rust));
    }
    if !clippy.is_empty() {
        sections.push(render_table("[lints.clippy]", clippy));
    }
    sections.push(CARGO_LINTS_BLOCK.to_string());
    sections.join("\n\n")
}

fn render_table(header: &str, table: &BTreeMap<String, toml::Value>) -> String {
    let lines: Vec<String> = table.iter().map(|(key, value)| format!("{key} = {value}")).collect();
    format!("{header}\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_no_configured_entries() {
        let cfg = CargoLintsConfig::default();
        assert!(cfg.is_empty());
    }

    /// Regression: an unconfigured `CargoLintsConfig` must still render the built-in
    /// `[lints.clippy]` deny block — `is_empty()` describing "nothing configured" must not be
    /// read as "nothing rendered", which was exactly the coverage-loss bug (four binding
    /// crates' hand-added `[lints.clippy]` block silently dropped by a full regen) this
    /// built-in closes.
    #[test]
    fn render_emits_the_builtin_clippy_block_when_nothing_is_configured() {
        let cfg = CargoLintsConfig::default();
        assert_eq!(
            cfg.render(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n\n[lints.cargo]\nunused_dependencies = \"allow\""
        );
    }

    #[test]
    fn deserializes_string_and_table_values() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
            print_stderr = "deny"
            dbg_macro = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert!(!cfg.is_empty());
        assert_eq!(cfg.rust.len(), 1);
        assert_eq!(cfg.clippy.len(), 3);
        assert_eq!(cfg.rust["unused_must_use"].as_str(), Some("deny"));
    }

    #[test]
    fn render_emits_both_tables_sorted_and_no_trailing_newline() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
            dbg_macro = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let rendered = cfg.render();
        assert_eq!(
            rendered,
            "[lints.rust]\nunused_must_use = \"deny\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\n\
             print_stderr = \"deny\"\nprint_stdout = \"deny\"\n\n[lints.cargo]\nunused_dependencies = \"allow\""
        );
    }

    #[test]
    fn render_omits_absent_rust_table_and_merges_builtin_clippy_defaults() {
        let toml_src = r#"
            [clippy]
            print_stdout = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(
            cfg.render(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n\n[lints.cargo]\nunused_dependencies = \"allow\""
        );
    }

    /// Regression: a configured `clippy` entry for a non-baseline key (something
    /// [`BUILTIN_CLIPPY_DEFAULTS`] never mentions) must survive the merge alongside the
    /// three built-in entries — the built-in must be additive, never a replacement for
    /// whatever the consumer configured.
    #[test]
    fn render_keeps_a_non_builtin_configured_clippy_key_alongside_the_builtin_defaults() {
        let toml_src = r#"
            [clippy]
            unwrap_used = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(
            cfg.render(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n\
             unwrap_used = \"warn\"\n\n[lints.cargo]\nunused_dependencies = \"allow\""
        );
    }

    /// Regression: a consumer's own value for a built-in key must win over the built-in
    /// default -- the built-in guarantees the key is *present*, not that its value is
    /// fixed, so a crate with a real reason to relax one of the three denies still can.
    #[test]
    fn render_lets_a_configured_value_override_the_builtin_default_for_the_same_key() {
        let toml_src = r#"
            [clippy]
            print_stdout = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(
            cfg.render(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"warn\"\n\n[lints.cargo]\nunused_dependencies = \"allow\""
        );
    }

    #[test]
    fn render_accepts_table_valued_entries() {
        let toml_src = r#"
            [rust.unexpected_cfgs]
            level = "warn"
            check-cfg = ["cfg(docsrs)"]
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let rendered = cfg.render();
        assert!(rendered.starts_with("[lints.rust]\nunexpected_cfgs = "));
        assert!(rendered.contains("level = \"warn\""));
        assert!(rendered.contains("check-cfg = [\"cfg(docsrs)\"]"));
        toml::from_str::<toml::Value>(&format!("[a]\n{rendered}\n")).expect("rendered entry must itself be valid TOML");
    }

    #[test]
    fn extra_rust_lines_excludes_the_builtin_key_and_sorts_the_rest() {
        let toml_src = r#"
            [rust]
            unexpected_cfgs = "warn"
            unused_must_use = "deny"
            non_snake_case = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let lines = cfg.extra_rust_lines(&["unexpected_cfgs"]);
        assert_eq!(
            lines,
            vec![
                "non_snake_case = \"warn\"".to_string(),
                "unused_must_use = \"deny\"".to_string(),
            ],
            "the excluded key must be dropped and the rest sorted by key"
        );
    }

    #[test]
    fn extra_rust_lines_is_empty_when_only_the_excluded_key_is_set() {
        let toml_src = r#"
            [rust]
            unexpected_cfgs = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert!(cfg.extra_rust_lines(&["unexpected_cfgs"]).is_empty());
    }

    #[test]
    fn clippy_block_renders_the_clippy_table_alone_merged_with_builtin_defaults() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(
            cfg.clippy_block(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""
        );
    }

    #[test]
    fn clippy_block_renders_the_builtin_defaults_when_clippy_table_unset() {
        let cfg = CargoLintsConfig::default();
        assert_eq!(
            cfg.clippy_block(),
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""
        );
    }
}
