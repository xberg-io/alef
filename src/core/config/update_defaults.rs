use super::extras::Language;
use super::output::{StringOrVec, UpdateConfig};
use super::tools::{LangContext, require_ruby_bundler, require_tool, ruby_bundle};

fn ruby_update_command(output_dir: &str) -> String {
    let get_frozen = ruby_bundle("config get frozen");
    let unfreeze = ruby_bundle("config set --local frozen false");
    let update = ruby_bundle("update --all");
    let restore = ruby_bundle("config set --local frozen \"$prev_frozen\"");
    format!(
        "cd {output_dir} && prev_frozen=$({get_frozen} 2>/dev/null | awk '/Set for your local app/ {{print $NF}}'); {unfreeze}; {update}; status=$?; if [ -n \"$prev_frozen\" ] && [ \"$prev_frozen\" != \"false\" ]; then {restore}; fi; exit $status"
    )
}

/// The shell variable name `maven_version_rules_assignment` computes the optional
/// `-Dmaven.version.rules=…` flag into, and `MAVEN_VERSION_RULES_REF` reads back -- the two must
/// name the same variable. Namespaced so it cannot collide with a variable the user's own shell
/// environment happens to export; it lives only for the lifetime of the one `sh -c` invocation
/// that sets and reads it.
const MAVEN_VERSION_RULES_VAR: &str = "alef_mvn_rules";

/// A *statement* (not an expression) that computes the optional `-Dmaven.version.rules=…`
/// argument into `$alef_mvn_rules`, emitted only when the scaffolded `versions-rules.xml` is
/// actually on disk. Must be joined onto the front of the `mvn` invocation with `;` — see
/// `MAVEN_VERSION_RULES_REF` for why the flag cannot be interpolated directly as a `$(...)`
/// expression at the call site.
///
/// `output_dir` arrives already shell-quoted (`'packages/java'`), and that is exactly why this
/// fragment cannot be written as one flat double-quoted string: inside `echo "…"` a single quote
/// is a *literal character*, not a quoting operator, so interpolating there emits
/// `file:///repo/'packages/java'/versions-rules.xml` — a URI with apostrophes in it that names no
/// file, silently disarming the rules maven was told to read. The interpolation therefore closes
/// the double quotes around it (`…/"{output_dir}"/versions-rules.xml`) so the already-quoted word
/// concatenates into the same argument while still being quoted *by the shell*, which keeps a
/// `$(…)` or backtick in a configured output path inert. Verified against a real `sh`, not by
/// reading. ~keep
fn maven_version_rules_assignment(output_dir: &str) -> String {
    format!(
        "{MAVEN_VERSION_RULES_VAR}=$([ -f {output_dir}/versions-rules.xml ] && echo \"-Dmaven.version.rules=file://${{PWD}}/\"{output_dir}\"/versions-rules.xml\")"
    )
}

/// Reference to the flag `maven_version_rules_assignment` computed, embedded directly as a word
/// in the `mvn` invocation. Must name `MAVEN_VERSION_RULES_VAR`.
///
/// `${var:+"$var"}` is the POSIX idiom for a shell word that is exactly one argument when `var`
/// is set and non-empty, and exactly *zero* when it is unset or empty. That double property is
/// why this cannot be simplified to `$(...)` or a bare `$var` at the call site (either word-splits
/// the captured value on any whitespace -- a `$PWD` or configured output path containing a space
/// would otherwise fracture one flag into several argv entries reaching maven) nor to a flat
/// `"$var"` (which would still hand maven one empty argument when no rules file exists, instead of
/// omitting the word entirely). Verified against a real `sh`, counting argv entries rather than
/// reading text, for both the with-space and no-rules-file cases. ~keep
const MAVEN_VERSION_RULES_REF: &str = r#"${alef_mvn_rules:+"$alef_mvn_rules"}"#;

/// Return the default update configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides the package manager selection.
pub fn default_update_config(lang: Language, output_dir: &str, ctx: &LangContext) -> UpdateConfig {
    let output_dir = super::shell::quote_word(output_dir);
    match lang {
        Language::Rust => UpdateConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            update: Some(StringOrVec::Single("cargo update".to_string())),
            upgrade: Some(StringOrVec::Multiple(vec![
                "cargo upgrade --incompatible".to_string(),
                "cargo update".to_string(),
            ])),
        },
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let (update_cmd, upgrade_cmd) = match pm {
                "pip" => (
                    format!("cd {output_dir} && pip install -U -e ."),
                    format!("cd {output_dir} && pip install -U -e ."),
                ),
                "poetry" => (
                    format!("cd {output_dir} && poetry update"),
                    format!("cd {output_dir} && poetry update --with dev"),
                ),
                _ => (
                    format!("cd {output_dir} && uv sync --upgrade --no-install-project --no-install-workspace"),
                    format!(
                        "cd {output_dir} && uv sync --all-packages --all-extras --upgrade --no-install-project --no-install-workspace"
                    ),
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Single(update_cmd)),
                upgrade: Some(StringOrVec::Single(upgrade_cmd)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let (update_cmds, upgrade_cmds) = match pm {
                "npm" => (
                    vec![format!("cd {output_dir} && npm update")],
                    vec![format!(
                        "cd {output_dir} && npm install -g npm-check-updates && ncu -u && npm install"
                    )],
                ),
                "yarn" => (
                    vec![format!("cd {output_dir} && yarn upgrade")],
                    vec![format!("cd {output_dir} && yarn upgrade --latest")],
                ),
                _ => (
                    // `--config.auto-install-peers=false --config.dedupe-peer-dependents=false`:
                    // without these, `pnpm up` promotes optional peer deps of installed packages
                    // (e.g. napi-rs's @emnapi/*, @octokit/core, typanion) into the project's own
                    // `dependencies` and stamps them with the workspace version — corrupting
                    // package.json on every update. ~keep
                    vec![
                        "corepack up".to_string(),
                        "pnpm up -r --config.auto-install-peers=false --config.dedupe-peer-dependents=false"
                            .to_string(),
                    ],
                    vec![
                        "corepack use pnpm@latest".to_string(),
                        "pnpm up --latest -r -w --config.auto-install-peers=false --config.dedupe-peer-dependents=false"
                            .to_string(),
                    ],
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Multiple(update_cmds)),
                upgrade: Some(StringOrVec::Multiple(upgrade_cmds)),
            }
        }
        Language::Ruby => {
            let command = ruby_update_command(&output_dir);
            UpdateConfig {
                precondition: Some(require_ruby_bundler()),
                before: None,
                update: Some(StringOrVec::Single(command.clone())),
                upgrade: Some(StringOrVec::Single(command)),
            }
        }
        Language::Php => UpdateConfig {
            precondition: Some(require_tool("composer")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && composer update"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer update --with-all-dependencies"
            ))),
        },
        Language::Go => UpdateConfig {
            precondition: Some(require_tool("go")),
            before: None,
            update: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
            upgrade: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
        },
        Language::Java => {
            let rules_assignment = maven_version_rules_assignment(&output_dir);
            let rules_ref = MAVEN_VERSION_RULES_REF;
            UpdateConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                update: Some(StringOrVec::Single(format!(
                    "{rules_assignment}; mvn -f {output_dir}/pom.xml versions:use-latest-releases {rules_ref} \
                     --batch-mode --no-transfer-progress"
                ))),
                upgrade: Some(StringOrVec::Single(format!(
                    "{rules_assignment}; mvn -f {output_dir}/pom.xml versions:use-latest-releases \
                     -DallowMajorUpdates=true {rules_ref} --batch-mode --no-transfer-progress"
                ))),
            }
        }
        Language::Csharp => UpdateConfig {
            precondition: Some(format!(
                "command -v dotnet >/dev/null 2>&1 && [ -n \"$(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)\" ]"
            )),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade --version-lock major $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
            ))),
        },
        Language::Elixir => UpdateConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
        },
        Language::R => UpdateConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
        },
        Language::Ffi => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
        Language::C => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
        // `dependencyUpdates` is provided by ben-manes/gradle-versions-plugin, not by Gradle
        // itself. Releases through 0.54.0 predate the plugin's Gradle 9 rework: v0.53.0's own
        // release notes say the task "fails ... in Gradle 9+ if run without disabling parallel
        // project execution", and v0.55.0 fixed that structurally by aggregating results from
        // per-project tasks instead of resolving them from the root project -- which is also
        // where the plugin's floor became Gradle 8.4 and its coordinate moved from
        // `com.github.ben-manes` to `io.github.ben-manes`. That is the real incompatibility four
        // of five consumer repos hit and papered over with a no-op override: alef's own
        // `kotlin_android` scaffold (`gen_build_gradle.rs`) pinned the plugin at 0.54.0, the last
        // pre-fix release. The fix is the version pin in
        // `template_versions::maven::GRADLE_VERSIONS_PLUGIN`, bumped to 0.61.0 (Gradle 9.7.x
        // compatible per the plugin's compatibility table) -- not this command line. The task
        // name (`dependencyUpdates`) and the Gradle Plugin Portal plugin id
        // (`com.github.ben-manes.versions`) are unaffected by the coordinate rename: the Portal
        // still publishes the old id as a redirect POM through the current release (verified
        // against https://plugins.gradle.org/m2/com/github/ben-manes/versions/... , not assumed),
        // so the existing scaffold resolves the fixed plugin once the pin is bumped, with no
        // backend change needed. Do not "fix" Gradle-9 breakage here by reverting to a no-op or a
        // `precondition = "false"` -- that is the defect this default exists to remove. ~keep
        Language::Kotlin | Language::KotlinAndroid => UpdateConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && gradle dependencyUpdates"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && gradle dependencyUpdates --refresh-dependencies"
            ))),
        },
        Language::Swift => UpdateConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "swift package update --package-path {output_dir}"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "swift package update --package-path {output_dir}"
            ))),
        },
        Language::Dart => UpdateConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && dart pub upgrade"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && dart pub upgrade --major-versions"
            ))),
        },
        Language::Zig => UpdateConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && zig build --fetch"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && zig build --fetch"))),
        },
        Language::Gleam => UpdateConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && gleam deps update"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && gleam deps update"))),
        },
        Language::Jni => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::ToolsConfig;
    use super::*;

    fn all_languages() -> Vec<Language> {
        vec![
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
            Language::Ffi,
            Language::Rust,
            Language::Kotlin,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
        ]
    }

    fn cfg(lang: Language, dir: &str) -> UpdateConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_update_config(lang, dir, &ctx)
    }

    #[test]
    fn generated_update_quotes_configured_output_directory() {
        let malicious = "packages/python; touch /tmp/alef-update; #";
        let commands = cfg(Language::Python, malicious)
            .update
            .expect("python update command")
            .commands()
            .join(" ");
        assert!(commands.contains(&format!("cd {}", super::super::shell::quote_word(malicious))));
    }

    #[test]
    fn ffi_has_no_update_commands() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.update.is_none());
        assert!(c.upgrade.is_none());
    }

    #[test]
    fn non_ffi_languages_have_update_commands() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.update.is_some(), "{lang} should have a default update command");
            assert!(c.upgrade.is_some(), "{lang} should have a default upgrade command");
        }
    }

    #[test]
    fn ruby_update_uses_the_active_interpreter_and_bundler() {
        let config = cfg(Language::Ruby, "packages/ruby");
        assert_eq!(
            config.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        let update = config.update.expect("ruby update command").commands().join(" ");
        let upgrade = config.upgrade.expect("ruby upgrade command").commands().join(" ");
        for command in [update, upgrade] {
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle config get frozen"),
                "got: {command}"
            );
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle config set --local frozen false"),
                "got: {command}"
            );
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle update --all"),
                "got: {command}"
            );
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn rust_update_uses_cargo() {
        let c = cfg(Language::Rust, "packages/rust");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("cargo update"));
        assert!(upgrade.contains("cargo upgrade --incompatible"));
        assert!(upgrade.contains("cargo update"));
    }

    #[test]
    fn rust_upgrade_is_multi_command() {
        let c = cfg(Language::Rust, "packages/rust");
        let upgrade = c.upgrade.unwrap();
        let cmds = upgrade.commands();
        assert!(cmds.len() >= 2);
    }

    #[test]
    fn python_update_uses_uv_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("uv sync"));
        assert!(upgrade.contains("--all-packages"));
        assert!(update.contains("--no-install-project"));
        assert!(upgrade.contains("--no-install-project"));
        assert!(update.contains("--no-install-workspace"));
        assert!(upgrade.contains("--no-install-workspace"));
    }

    #[test]
    fn python_update_dispatches_on_package_manager() {
        for (pm, expected) in [("pip", "pip install -U"), ("poetry", "poetry update")] {
            let tools = ToolsConfig {
                python_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Python, "packages/python", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn node_update_uses_pnpm_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("pnpm up"));
        assert!(upgrade.contains("pnpm up --latest"));
        // Both flags are required to stop `pnpm up` from promoting optional peer deps
        // into package.json with the workspace version stamped on them.
        for cmds in [&update, &upgrade] {
            assert!(cmds.contains("--config.auto-install-peers=false"));
            assert!(cmds.contains("--config.dedupe-peer-dependents=false"));
        }
    }

    #[test]
    fn node_update_dispatches_on_package_manager() {
        for (pm, expected) in [("npm", "npm update"), ("yarn", "yarn upgrade")] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Node, "packages/node", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    /// The directory as it is spelled *inside the emitted shell command* — a quoted word, not a
    /// bare path. Expectations derive it from `quote_word` rather than restating one quoting
    /// spelling, so a change to the escaping policy cannot silently repoint a command at a
    /// different directory: the escaping is proved separately, and once, by
    /// `shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile value
    /// through a real shell. ~keep
    fn quoted(dir: &str) -> String {
        super::super::shell::quote_word(dir)
    }

    #[test]
    fn java_update_uses_maven_versions() {
        let c = cfg(Language::Java, "packages/java");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("versions:use-latest-releases"));
        assert!(upgrade.contains("allowMajorUpdates=true"));
        assert!(
            update.contains(&format!("[ -f {}/versions-rules.xml ]", quoted("packages/java"))),
            "java update should make versions-rules.xml optional, got: {update}"
        );
    }

    /// Runs the assignment + `${var:+"$var"}` reference pair through a real `sh`, in `cwd`, and
    /// returns the rules flag as however many argv words it actually produced -- one per printed
    /// line, via `for w in REF; do printf '%s\n' "$w"; done`. Counting entries (not joining them
    /// back into one string) is the point: a joined comparison is exactly what would hide a
    /// `$PWD` or output-dir space fracturing the flag into more than one word before maven ever
    /// sees it. ~keep
    #[cfg(unix)]
    fn rules_flag_argv(output_dir_quoted: &str, cwd: &std::path::Path) -> Vec<String> {
        let assignment = maven_version_rules_assignment(output_dir_quoted);
        let script = format!("{assignment}; for w in {MAVEN_VERSION_RULES_REF}; do printf '%s\\n' \"$w\"; done");
        let output = std::process::Command::new("sh")
            .args(["-c", &script])
            .current_dir(cwd)
            .output()
            .expect("shell should start");
        assert!(
            output.status.success(),
            "the emitted fragment must be valid shell: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Cardinality matrix for the optional `-Dmaven.version.rules=…` flag: it must reach maven as
    /// exactly one argv entry whenever the scaffolded rules file exists (regardless of whitespace
    /// in the configured output path or in `$PWD`), and exactly zero when it does not. A prior
    /// version of this fragment used a bare `$(...)` at the call site, which word-splits its
    /// captured value on any whitespace -- case (b) below is the one that fragment got wrong: a
    /// path with a space in it fractured one flag into two argv entries reaching maven. ~keep
    #[cfg(unix)]
    #[test]
    fn maven_version_rules_flag_reaches_maven_as_exactly_one_or_zero_argv() {
        // (a) rules file exists, no spaces anywhere -> exactly 1 argv.
        {
            const PACKAGE: &str = "packages/java";
            let root = tempfile::tempdir().expect("tempdir");
            let package = root.path().join(PACKAGE);
            std::fs::create_dir_all(&package).expect("create package dir");
            std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");

            let argv = rules_flag_argv(&quoted(PACKAGE), root.path());
            assert_eq!(argv.len(), 1, "expected exactly 1 argv entry, got {argv:?}");
            let uri = argv[0]
                .strip_prefix("-Dmaven.version.rules=file://")
                .unwrap_or_else(|| panic!("expected a file:// rules URI, got {argv:?}"));
            assert!(
                std::path::Path::new(uri).is_file(),
                "maven is pointed at `{uri}`, which is not the rules file that exists on disk"
            );
        }

        // (b) rules file exists, output dir path contains a space -> exactly 1 argv (this is the
        // case the pre-fix bare `$(...)` fractured into 2).
        {
            const PACKAGE: &str = "packages/my java";
            let root = tempfile::tempdir().expect("tempdir");
            let package = root.path().join(PACKAGE);
            std::fs::create_dir_all(&package).expect("create package dir");
            std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");

            let argv = rules_flag_argv(&quoted(PACKAGE), root.path());
            assert_eq!(
                argv.len(),
                1,
                "a space in the configured output path must not fracture the flag: got {argv:?}"
            );
            let uri = argv[0]
                .strip_prefix("-Dmaven.version.rules=file://")
                .unwrap_or_else(|| panic!("expected a file:// rules URI, got {argv:?}"));
            assert!(
                std::path::Path::new(uri).is_file(),
                "maven is pointed at `{uri}`, which is not the rules file that exists on disk"
            );
        }

        {
            const PACKAGE: &str = "packages/java";
            let root = tempfile::tempdir().expect("tempdir");
            std::fs::create_dir_all(root.path().join(PACKAGE)).expect("create package dir");

            let argv = rules_flag_argv(&quoted(PACKAGE), root.path());
            assert!(
                argv.is_empty(),
                "no rules file on disk must emit zero argv entries, not an empty one: got {argv:?}"
            );
        }

        // (d) rules file exists, no space in the output dir but `$PWD` itself contains one ->
        // exactly 1 argv.
        {
            const PACKAGE: &str = "packages/java";
            let root = tempfile::tempdir().expect("tempdir");
            let space_root = root.path().join("space dir");
            let package = space_root.join(PACKAGE);
            std::fs::create_dir_all(&package).expect("create package dir");
            std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");

            let argv = rules_flag_argv(&quoted(PACKAGE), &space_root);
            assert_eq!(
                argv.len(),
                1,
                "a space in $PWD must not fracture the flag: got {argv:?}"
            );
            let uri = argv[0]
                .strip_prefix("-Dmaven.version.rules=file://")
                .unwrap_or_else(|| panic!("expected a file:// rules URI, got {argv:?}"));
            assert!(
                std::path::Path::new(uri).is_file(),
                "maven is pointed at `{uri}`, which is not the rules file that exists on disk"
            );
        }
    }

    /// A configured output path is consumer input reaching `sh -c`. Inside the `echo "…"` that
    /// builds the rules URI, `;` is inert but `$(…)` is not — so the check that matters is that
    /// no command substitution runs, not that no semicolon survives. ~keep
    #[cfg(unix)]
    #[test]
    fn java_version_rules_flag_does_not_execute_a_configured_output_path() {
        // The hostile directory and its rules file must really exist, or the `[ -f … ]` guard
        // short-circuits and the `echo` this test exists to exercise never runs — the check
        // would then pass while examining nothing. ~keep
        const HOSTILE: &str = "packages/java$(touch executed)";
        let root = tempfile::tempdir().expect("tempdir");
        let package = root.path().join(HOSTILE);
        std::fs::create_dir_all(&package).expect("create hostile package dir");
        std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");
        let witness = root.path().join("executed");

        let argv = rules_flag_argv(&quoted(HOSTILE), root.path());

        assert_eq!(argv.len(), 1, "the rules flag must have been emitted, got {argv:?}");
        assert!(
            !witness.exists(),
            "a command substitution in the configured output path was executed by the update command"
        );
    }

    #[test]
    fn csharp_update_resolves_csproj_in_subdir() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        let find = format!("find {}", quoted("packages/csharp"));
        assert!(update.contains(&find), "update should locate csproj, got: {update}");
        assert!(upgrade.contains(&find), "upgrade should locate csproj, got: {upgrade}");
    }

    #[test]
    fn csharp_precondition_requires_project_file() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let pre = c.precondition.unwrap();
        assert!(
            pre.contains(&format!("find {}", quoted("packages/csharp"))),
            "precondition should search for project file, got: {pre}"
        );
        assert!(pre.contains("dotnet"), "precondition should still require dotnet CLI");
    }

    #[test]
    fn output_dir_substituted_in_update_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let update = c.update.unwrap().commands().join(" ");
        assert!(update.contains("my/custom/path"));
    }

    #[test]
    fn r_update_is_non_interactive() {
        let c = cfg(Language::R, "packages/r");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("ask = FALSE"), "R update must be non-interactive");
        assert!(upgrade.contains("ask = FALSE"), "R upgrade must be non-interactive");
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        let node_update = node.update.unwrap().commands().join(" ");
        let wasm_update = wasm.update.unwrap().commands().join(" ");
        assert_eq!(node_update, wasm_update);
    }

    #[test]
    fn kotlin_uses_gradle_dependency_updates() {
        let c = cfg(Language::Kotlin, "packages/kotlin");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("gradle dependencyUpdates"),
            "Kotlin update should use gradle dependencyUpdates, got: {update}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    /// `Language::Kotlin` and `Language::KotlinAndroid` share one match arm in
    /// `default_update_config`; this exercises the arm through the `KotlinAndroid` variant
    /// specifically, substitutes a distinctive `output_dir`, and checks `upgrade` carries the
    /// `--refresh-dependencies` flag `update` must not have -- so a future edit that collapses
    /// them back to one shared command (the no-op shape this default exists to avoid) or drops the
    /// output_dir substitution fails here, not silently. ~keep
    #[test]
    fn kotlin_android_update_and_upgrade_substitute_output_dir() {
        let c = cfg(Language::KotlinAndroid, "my/custom/kotlin-dir");
        let update = c.update.expect("kotlin_android update command").commands().join(" ");
        let upgrade = c.upgrade.expect("kotlin_android upgrade command").commands().join(" ");

        assert!(
            update.contains("gradle dependencyUpdates") && update.contains("my/custom/kotlin-dir"),
            "got: {update}"
        );
        assert!(
            upgrade.contains("gradle dependencyUpdates --refresh-dependencies")
                && upgrade.contains("my/custom/kotlin-dir"),
            "got: {upgrade}"
        );
        assert!(
            !update.contains("--refresh-dependencies"),
            "update must not carry the upgrade-only refresh flag, got: {update}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_package_update() {
        let c = cfg(Language::Swift, "packages/swift");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("swift package update"),
            "Swift update should use swift package update, got: {update}"
        );
        assert!(
            update.contains(&format!("--package-path {}", quoted("packages/swift"))),
            "Swift update should include package path, got: {update}"
        );
    }

    #[test]
    fn dart_uses_dart_pub_upgrade() {
        let c = cfg(Language::Dart, "packages/dart");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(
            update.contains("dart pub upgrade"),
            "Dart update should use dart pub upgrade, got: {update}"
        );
        assert!(
            upgrade.contains("--major-versions"),
            "Dart upgrade should include --major-versions, got: {upgrade}"
        );
    }

    #[test]
    fn gleam_uses_gleam_deps_update() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(
            update.contains("gleam deps update"),
            "Gleam update should use gleam deps update, got: {update}"
        );
        assert!(
            upgrade.contains("gleam deps update"),
            "Gleam upgrade should use gleam deps update, got: {upgrade}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build_fetch() {
        let c = cfg(Language::Zig, "packages/zig");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("zig build --fetch"),
            "Zig update should use zig build --fetch, got: {update}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
