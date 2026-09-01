//! Validation of user-supplied pipeline overrides in `alef.toml`.
//!
//! `test` is the only remaining per-command override table (0.82.0 removed
//! `lint`/`build_commands`/`setup`/`update`/`clean` from the schema entirely -- alef now owns
//! those commands end to end). When a user provides an explicit `[test.<lang>]` table that
//! **sets a main command field**, that table must also declare a `precondition`. The rationale:
//!
//! - Built-in defaults all declare a `command -v <tool>` precondition so
//!   pipelines degrade gracefully when the underlying tool is missing.
//! - A custom `test` command is opaque to alef — only the user knows what the
//!   command requires. Forcing an explicit `precondition` keeps the
//!   warn-and-skip behavior intact on systems that can't run the command.
//!
//! A table that only customizes `before` (without overriding the main command)
//! is exempt: the default precondition still applies via the surrounding
//! defaults logic.

mod preconditions;

use super::extras::Language;
use super::output::validate_output_segment;
use super::resolved::ResolvedCrateConfig;
use crate::core::error::AlefError;
use preconditions::{test_main_fields, validate_section, validate_test_e2e_precondition, validate_tools};

/// Validate user-supplied pipeline overrides in a resolved per-crate config.
///
/// Operates on the merged pipeline maps (already `HashMap` rather than
/// `Option<HashMap>`) that `ResolvedCrateConfig` carries after workspace
/// defaults are folded in.
pub fn validate_resolved(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    validate_tools(&config.tools)?;
    validate_package_metadata(config)?;
    validate_e2e_env_keys(config)?;
    validate_extra_lint_paths(config)?;
    validate_section("test", &config.test, test_main_fields, |c| c.precondition.as_deref())?;
    validate_test_e2e_precondition(&config.test)?;
    validate_trait_bridges(config)?;
    validate_dart_library_name(config)?;
    Ok(())
}

/// Reject a derived Dart library/barrel-file name that would escape the output tree once
/// `DartBackend::generate_bindings` writes it.
///
/// `[dart] lib_name` is validated at config-resolution time when explicitly set (see
/// `new_config::validate_path_segment_field`), but when it is unset the effective name falls
/// back through `dart_pubspec_name()` to the crate name with only hyphens replaced —
/// unlike the reverse-DNS package derivation Java/Kotlin fall back to, that fallback does not
/// strip path separators from an unusual crate `name`. There is no single config key to name
/// in that case (the value comes from a chain of defaults, not one field), so this check runs
/// against the resolved, post-default value here rather than in `resolve_one`.
fn validate_dart_library_name(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    if !config.targets(Language::Dart) {
        return Ok(());
    }
    validate_output_segment(&config.dart_library_name(), "dart.lib_name (derived from crate name)")
        .map_err(|detail| AlefError::Config(format!("crate `{}`: {detail}", config.name)))
}

/// Reject a trait bridge that declares a registration function it cannot emit.
///
/// `registry_getter` is `Option` on the config struct, but the FFI backend's registration
/// emitter needs it and `expect`s it — so a bridge with `register_fn` and no `registry_getter`
/// passed validation, survived extraction, and panicked several stages later inside binding
/// generation, naming an internal function rather than the config key at fault. Checking it here
/// fails at load with the bridge named, before any file is written. ~keep
fn validate_trait_bridges(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    for bridge in &config.trait_bridges {
        if bridge.register_fn.is_some() && bridge.registry_getter.is_none() {
            return Err(AlefError::Config(format!(
                "trait bridge `{}` sets `register_fn` but no `registry_getter`. Add `registry_getter` \
                 to `[[crates.trait_bridges]]` for `{}`, or drop `register_fn`.",
                bridge.trait_name, bridge.trait_name
            )));
        }
    }
    Ok(())
}

/// Whether `name` is a valid POSIX-style environment variable name (`[A-Za-z_][A-Za-z0-9_]*`).
///
/// The sole gate `[crates.e2e.env]` keys pass through, once, at config resolution. That single
/// point of control covers every downstream consumer of `config.e2e.env` at once: the shell
/// sites that fold it into a command string (`cli::pipeline::commands::test_apps`,
/// `cli::pipeline::commands::test`) and the generated-output sites that serialize it into a
/// target language's own literal syntax (Elixir, Ruby, TypeScript, Rust, WASM JS harnesses).
/// A key confined to this pattern carries nothing any of those grammars could reinterpret --
/// no shell metacharacter, no quote, no interpolation marker -- so no downstream consumer needs
/// its own key-side defense. Values are deliberately unrestricted: they are meant to carry
/// arbitrary text, so each consumer remains responsible for encoding a *value* safely for its
/// own target (shell sites via `Command::env`, generated-output sites via their language's own
/// escaping).
pub(crate) fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Reject a `[crates.e2e.env]` key that is not a valid POSIX-style environment variable name.
///
/// See [`is_valid_env_var_name`] for why this is the single highest-leverage check available:
/// every consumer of `config.e2e.env`, shell-based or generated-output, receives an
/// already-safe key once this passes. Rejecting at config resolution (rather than warning and
/// dropping the entry downstream) is deliberate: an invalid key here is a config-authoring
/// mistake the operator needs to see and fix, not a hazard the pipeline should quietly work
/// around by discarding the variable a fixture may depend on.
fn validate_e2e_env_keys(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    let Some(e2e) = &config.e2e else {
        return Ok(());
    };
    let mut invalid: Vec<&str> = e2e
        .env
        .keys()
        .map(String::as_str)
        .filter(|key| !is_valid_env_var_name(key))
        .collect();
    if invalid.is_empty() {
        return Ok(());
    }
    invalid.sort_unstable();
    Err(AlefError::Config(format!(
        "invalid `[crates.e2e.env]` key(s) for crate `{}`: {}. Environment variable names must \
         match `[A-Za-z_][A-Za-z0-9_]*` -- they are forwarded into shell commands and into \
         several generated languages' own source, and a name outside this pattern cannot be \
         expressed safely in all of them.",
        config.name,
        invalid.join(", ")
    )))
}

/// Whether `path` is a well-formed path fragment (`[A-Za-z0-9._/-]+`, non-empty).
///
/// `[crates.<lang>].extra_lint_paths` entries reach
/// [`crate::core::config::tools::append_paths`], which space-joins and appends them to a
/// lint/format shell command string with no quoting at all -- there is no `Command::env` or
/// argv boundary available at that call site the way there is for `[go] module` or a clean
/// command's `output_dir`, because `append_paths` composes plain shell text meant for
/// `run_command`/`run_command_streamed`, not `ArgvRunConfig`. A path confined to this pattern
/// carries nothing that grammar could reinterpret: no whitespace to split off an extra
/// argument, no `;`/backtick/`$(...)` to run a second command.
fn is_well_formed_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')
}

/// Reject an `extra_lint_paths` entry that is not a well-formed path fragment.
///
/// See [`is_well_formed_path_char`] for why this is the single choke point: every
/// `append_paths` call site (one per lint/format/typecheck default across every language)
/// receives an already-safe path once this passes, without each call site needing its own
/// escaping.
fn validate_extra_lint_paths(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    let mut invalid: Vec<String> = Vec::new();
    for lang in Language::ALL {
        for path in config.extra_lint_paths_for_language(lang) {
            if path.is_empty() || !path.chars().all(is_well_formed_path_char) {
                invalid.push(format!("{lang}: {path:?}"));
            }
        }
    }
    if invalid.is_empty() {
        return Ok(());
    }
    Err(AlefError::Config(format!(
        "invalid `extra_lint_paths` entries for crate `{}`: {}. Entries are appended verbatim \
         into a lint/format shell command with no quoting, so they must match \
         `[A-Za-z0-9._/-]+` -- no whitespace or shell metacharacters.",
        config.name,
        invalid.join(", ")
    )))
}

fn validate_package_metadata(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    const CRATES_IO_LIST_LIMIT: usize = 5;
    let Some(meta) = &config.package_metadata else {
        return Ok(());
    };
    if !meta.truncate_registry_lists {
        if meta.keywords.len() > CRATES_IO_LIST_LIMIT {
            return Err(AlefError::Config(format!(
                "crate `{}` package_metadata.keywords has {} entries; crates.io supports at most {CRATES_IO_LIST_LIMIT}. \
                 Reduce the list or set package_metadata.truncate_registry_lists = true.",
                config.name,
                meta.keywords.len()
            )));
        }
        if meta.categories.len() > CRATES_IO_LIST_LIMIT {
            return Err(AlefError::Config(format!(
                "crate `{}` package_metadata.categories has {} entries; crates.io supports at most {CRATES_IO_LIST_LIMIT}. \
                 Reduce the list or set package_metadata.truncate_registry_lists = true.",
                config.name,
                meta.categories.len()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;

    /// Parse a new-schema alef.toml and return the first resolved crate.
    fn resolve_first(toml_str: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_str).expect("config should parse");
        cfg.resolve().expect("config should resolve").remove(0)
    }

    fn base_config() -> &'static str {
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#
    }

    #[test]
    fn no_user_overrides_is_valid() {
        let config = resolve_first(base_config());
        validate_resolved(&config).expect("default config should validate");
    }

    /// RED: an `[crates.e2e.env]` key carrying shell syntax must be rejected at config
    /// resolution, not silently forwarded to any downstream consumer.
    #[test]
    fn e2e_env_key_with_shell_syntax_is_rejected() {
        let toml = format!(
            "{base}\n[crates.e2e]\nfixtures = \"fixtures\"\noutput = \"e2e\"\n\
             [crates.e2e.call]\nfunction = \"process\"\nmodule = \"test-lib\"\n\
             [crates.e2e.env]\n\"MYVAR; touch pwned\" = \"safe\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("shell-shaped env key must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("MYVAR; touch pwned"),
            "error should name the bad key: {msg}"
        );
        assert!(
            msg.contains("[A-Za-z_][A-Za-z0-9_]*"),
            "error should name the required pattern: {msg}"
        );
    }

    /// GREEN: a conventional identifier-shaped env key validates cleanly.
    #[test]
    fn e2e_env_key_valid_identifier_is_accepted() {
        let toml = format!(
            "{base}\n[crates.e2e]\nfixtures = \"fixtures\"\noutput = \"e2e\"\n\
             [crates.e2e.call]\nfunction = \"process\"\nmodule = \"test-lib\"\n\
             [crates.e2e.env]\nALLOW_PRIVATE_NETWORK = \"true\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("identifier-shaped env key should validate");
    }

    /// RED: an `extra_lint_paths` entry carrying shell syntax must be rejected at config
    /// resolution, not silently appended verbatim to a lint/format shell command.
    #[test]
    fn extra_lint_paths_entry_with_shell_syntax_is_rejected() {
        let toml = format!(
            "{base}\n[crates.python]\nextra_lint_paths = [\"scripts; touch pwned\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("shell-shaped extra_lint_paths entry must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("scripts; touch pwned"),
            "error should name the bad entry: {msg}"
        );
        assert!(
            msg.contains("[A-Za-z0-9._/-]+"),
            "error should name the required pattern: {msg}"
        );
    }

    /// GREEN: a conventional path-shaped `extra_lint_paths` entry validates cleanly.
    #[test]
    fn extra_lint_paths_entry_valid_path_is_accepted() {
        let toml = format!(
            "{base}\n[crates.python]\nextra_lint_paths = [\"scripts/helpers.py\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("path-shaped extra_lint_paths entry should validate");
    }

    #[test]
    fn is_valid_env_var_name_accepts_identifiers_and_rejects_shell_syntax() {
        for ok in ["MOCK_SERVER_URL", "_leading_underscore", "A1", "a"] {
            assert!(is_valid_env_var_name(ok), "expected {ok:?} to be valid");
        }
        for bad in [
            "",
            "1LEADING_DIGIT",
            "HAS-HYPHEN",
            "HAS SPACE",
            "HAS;SEMI",
            "HAS'QUOTE",
            "$(cmd)",
        ] {
            assert!(!is_valid_env_var_name(bad), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn is_well_formed_path_char_accepts_paths_and_rejects_shell_syntax() {
        for ok in ["scripts/helpers.py", "a_b-c.d", "vendor/third_party"] {
            assert!(
                ok.chars().all(is_well_formed_path_char),
                "expected {ok:?} to be all well-formed path chars"
            );
        }
        for bad in [
            "scripts; touch pwned",
            "scripts`touch pwned`",
            "scripts$(touch pwned)",
            "has space",
            "has'quote",
        ] {
            assert!(
                !bad.chars().all(is_well_formed_path_char),
                "expected {bad:?} to contain a rejected char"
            );
        }
    }

    #[test]
    fn test_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.test.python]\ncommand = \"pytest\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("missing precondition should error");
        assert!(format!("{err}").contains("[test.python]"));
    }

    #[test]
    fn test_override_with_only_e2e_requires_precondition() {
        let toml = format!(
            "{base}\n[crates.test.python]\ne2e = \"pytest tests/e2e\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("e2e without precondition or e2e_precondition should error");
        let msg = format!("{err}");
        assert!(msg.contains("[test.python]"), "{msg}");
        assert!(msg.contains("e2e_precondition"), "{msg}");
    }

    #[test]
    fn test_override_with_only_e2e_and_e2e_precondition_is_ok() {
        let toml = format!(
            "{base}\n[crates.test.python]\ne2e_precondition = \"command -v uv\"\ne2e = \"pytest tests/e2e\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("e2e with e2e_precondition alone should validate");
    }

    #[test]
    fn test_override_with_e2e_and_command_needs_only_top_level_precondition() {
        let toml = format!(
            "{base}\n[crates.test.python]\nprecondition = \"command -v pytest\"\ncommand = \"pytest\"\ne2e = \
             \"pytest tests/e2e\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config)
            .expect("a top-level precondition still satisfies both command and e2e when no e2e_precondition is set");
    }

    #[test]
    fn error_message_lists_only_actually_set_main_fields() {
        let toml = format!(
            "{base}\n[crates.test.python]\ncommand = \"pytest\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let msg = format!("{}", validate_resolved(&config).unwrap_err());
        assert!(msg.contains("`command`"), "expected `command`, got: {msg}");
        assert!(
            !msg.contains("`coverage`"),
            "should not mention unset `coverage`: {msg}"
        );
    }

    #[test]
    fn before_plus_main_cmd_without_precondition_still_errors() {
        let toml = format!(
            "{base}\n[crates.test.python]\nbefore = \"echo hi\"\ncommand = \"pytest\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("before + main without precondition must error");
    }

    #[test]
    fn malformed_python_package_manager_value_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv; rm -rf /\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
        assert!(format!("{err}").contains("well-formed"));
    }

    #[test]
    fn malformed_node_package_manager_value_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\nnode_package_manager = \"pnpm$(echo bad)\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn malformed_rust_dev_tool_entry_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\nrust_dev_tools = [\"cargo-edit\", \"cargo`evil`\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn whitespace_in_tool_name_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv \"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("trailing whitespace must be rejected");
    }

    #[test]
    fn empty_tool_name_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("empty tool name must be rejected");
    }

    #[test]
    fn safe_tool_names_are_accepted() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv\"\n\
             node_package_manager = \"pnpm\"\n\
             rust_dev_tools = [\"cargo-edit\", \"cargo_sort\", \"tool.v2\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("normal tool names should validate");
    }

    #[test]
    fn package_metadata_keywords_over_crates_io_limit_errors() {
        let toml = format!(
            "{base}\n[crates.package_metadata]\nkeywords = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("too many crates.io keywords should error");
        let msg = format!("{err}");
        assert!(msg.contains("package_metadata.keywords"), "got: {msg}");
        assert!(msg.contains("at most 5"), "got: {msg}");
    }

    #[test]
    fn package_metadata_can_opt_into_registry_list_truncation() {
        let toml = format!(
            "{base}\n[crates.package_metadata]\n\
             truncate_registry_lists = true\n\
             keywords = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n\
             categories = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("explicit truncation opt-in should validate");
    }

    #[test]
    fn override_with_main_cmd_and_precondition_validates() {
        let toml = format!(
            "{base}\n[crates.test.python]\nprecondition = \"command -v tool\"\ncommand = \"tool run\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).unwrap_or_else(|e| panic!("[test] with precondition should validate: {e}"));
    }

    // -----------------------------------------------------------------------------------------
    // `validate_dart_library_name`: the "derived default with no config key to reject" case --
    // `[dart] lib_name` is checked at config-resolution time when explicitly set (see
    // `new_config::path_safety_tests`), but its fallback to the crate name (with only hyphens
    // replaced) is a value with no single field to name, so it is checked here instead against
    // the resolved config.
    // -----------------------------------------------------------------------------------------

    /// This used to exercise `validate_dart_library_name` below via the
    /// `dart_library_name()` -> `dart_pubspec_name()` -> `self.name.replace('-', "_")`
    /// fallback: an unconfigured crate whose `name` itself contained `/` reached this
    /// check and was rejected here. `validate_crate_name_path_safety`
    /// (`new_config::validate_crate_name_path_safety`) now runs unconditionally, for every
    /// crate regardless of target language, before any per-language resolution -- so a `/` in
    /// `name` is rejected by `resolve()` itself, and can no longer reach this function at all
    /// via the "unconfigured" fallback path. `resolve_first` below fails outright, which is the
    /// behavior this test now locks in; the still-live gap this check does close (an
    /// unconfigured Dart-targeted crate with a hazardous `[crates.dart].pubspec_name`, which
    /// `resolve()` does *not* validate unless Dart is targeted) is
    /// `dart_library_name_is_not_checked_for_a_crate_that_does_not_target_dart` below, which
    /// proves the opposite side of the same check with a value that still reaches it. ~keep
    #[test]
    fn crate_name_with_a_path_separator_is_rejected_before_the_dart_check_can_run() {
        let toml_str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample/evil"
sources = ["src/lib.rs"]

[crates.output]
dart = "packages/dart/lib/src"
"#;
        let cfg: NewAlefConfig = toml::from_str(toml_str).expect("config should parse");
        let err = cfg
            .resolve()
            .expect_err("a `/` in the crate name must be rejected at resolve time");
        let message = err.to_string();
        assert!(
            message.contains("sample/evil"),
            "error should name the crate: {message}"
        );
        assert!(
            message.contains("invalid name"),
            "error should point at the crate `name` field, not a Dart-specific one: {message}"
        );
        assert!(
            message.contains("path separators are not allowed"),
            "error should explain why: {message}"
        );
    }

    #[test]
    fn dart_library_name_accepts_an_ordinary_crate_name_when_unconfigured() {
        let toml_str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
"#;
        let config = resolve_first(toml_str);
        validate_resolved(&config).expect("an ordinary crate name must still validate");
    }

    #[test]
    fn dart_library_name_is_not_checked_for_a_crate_that_does_not_target_dart() {
        // The crate `name` itself is ordinary ("sample-core"): `validate_crate_name_path_safety`
        // now runs unconditionally regardless of target language (see the sibling test above),
        // so a hazardous *name* can no longer reach `resolve_first` at all here, let alone this
        // check. The hazard instead lives in `[crates.dart].pubspec_name` -- `resolve()` only
        // validates that field via `validate_dart_coordinates` when Dart is a targeted language
        // (see `new_config::validate_dart_coordinates`), so with `languages = ["python"]` it
        // sails through `resolve_first` unvalidated, and `dart_library_name()` returns it
        // verbatim. This still proves the behavior the test name claims: `validate_dart_library_name`
        // must skip a demonstrably hazardous value because the crate does not target Dart, not
        // because the value happened to be safe.
        let toml_str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample/evil"

[crates.output]
python = "packages/python"
"#;
        let config = resolve_first(toml_str);
        assert_eq!(
            config.dart_library_name(),
            "sample/evil",
            "fixture must carry the hazard through unvalidated"
        );
        validate_resolved(&config).expect("a crate not targeting dart must not be checked");
    }
}
