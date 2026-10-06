use super::*;
use crate::e2e::config::DependencyMode;
use std::collections::BTreeMap;

fn make_e2e_config_with_env(env: BTreeMap<String, String>) -> E2eConfig {
    E2eConfig {
        env,
        ..E2eConfig::default()
    }
}

fn bootstrap_with_harness(uses_server_harness: bool) -> String {
    render_bootstrap(BootstrapOptions {
        e2e_config: &E2eConfig::default(),
        pkg_path: "../../packages/php",
        has_mock_server_fixtures: false,
        has_file_fixtures: false,
        test_documents_path: "../../testing_data",
        uses_server_harness,
        harness_host: "127.0.0.1",
        harness_port: 8000,
    })
}

/// The harness runs as a child of a process started with `-n`, so it inherits no
/// php.ini and therefore no extension. Spawned bare it dies on the first `new App()`
/// with "Class not found", which reads as a missing autoloader rather than as a
/// deliberately isolated interpreter. Both halves are pinned here because either one
/// alone is silent: an exported path nothing reads, or a read of something never set.
#[test]
fn the_app_harness_is_spawned_with_the_extension_the_tests_verified() {
    let bootstrap = bootstrap_with_harness(true);
    assert!(
        bootstrap.contains("getenv('ALEF_PHP_EXTENSION_PATH')"),
        "bootstrap must read the exported extension path: {bootstrap}"
    );
    assert!(
        bootstrap.contains("'-d', 'extension=' . $harnessExtPath"),
        "bootstrap must pass the extension to the harness child: {bootstrap}"
    );
    assert!(
        !bootstrap.contains("proc_open([PHP_BINARY, $appHarnessBin]"),
        "the bare spawn is the defect and must not survive: {bootstrap}"
    );
}

/// Negative control: the export is worthless if the consumer never receives it, and a
/// project without a server harness must not grow a spawn it does not need.
#[test]
fn run_tests_exports_the_path_and_a_harnessless_project_spawns_nothing() {
    let run_tests = render_run_tests_php(
        "spikard_php",
        "spikard-php",
        "crates/spikard-php",
        "0.1.0",
        DependencyMode::Local,
    )
    .expect("run_tests renders");
    assert!(
        run_tests.contains("putenv('ALEF_PHP_EXTENSION_PATH=' . $extPath)"),
        "run_tests must export the verified extension path: {run_tests}"
    );
    assert!(
        !bootstrap_with_harness(false).contains("app_harness.php"),
        "a project with no server harness must not spawn one"
    );
}

#[test]
fn test_render_env_setup_block_empty_env() {
    let config = make_e2e_config_with_env(BTreeMap::new());
    let result = render_env_setup_block(&config);
    assert!(result.is_empty(), "empty env should produce empty setup block");
}

#[test]
fn test_render_env_setup_block_single_env_var() {
    let mut env = BTreeMap::new();
    env.insert("ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());

    let config = make_e2e_config_with_env(env);
    let result = render_env_setup_block(&config);

    assert!(
        result.contains("getenv('ALLOW_PRIVATE_NETWORK')"),
        "should check getenv"
    );
    assert!(
        result.contains("putenv('ALLOW_PRIVATE_NETWORK=true')"),
        "should call putenv"
    );
    assert!(
        result.contains("$_ENV['ALLOW_PRIVATE_NETWORK'] = 'true'"),
        "should set $_ENV"
    );
    assert!(
        result.contains("$_SERVER['ALLOW_PRIVATE_NETWORK'] = 'true'"),
        "should set $_SERVER"
    );
}

#[test]
fn test_render_env_setup_block_multiple_env_vars_sorted() {
    let mut env = BTreeMap::new();
    env.insert("ZEBRA_VAR".to_string(), "z_value".to_string());
    env.insert("ALPHA_VAR".to_string(), "a_value".to_string());
    env.insert("BETA_VAR".to_string(), "b_value".to_string());

    let config = make_e2e_config_with_env(env);
    let result = render_env_setup_block(&config);

    // Check that all variables are present
    assert!(result.contains("ALPHA_VAR"), "should contain ALPHA_VAR");
    assert!(result.contains("BETA_VAR"), "should contain BETA_VAR");
    assert!(result.contains("ZEBRA_VAR"), "should contain ZEBRA_VAR");

    // Check alphabetical ordering by verifying positions
    let alpha_pos = result.find("ALPHA_VAR").unwrap();
    let beta_pos = result.find("BETA_VAR").unwrap();
    let zebra_pos = result.find("ZEBRA_VAR").unwrap();

    assert!(alpha_pos < beta_pos, "ALPHA_VAR should appear before BETA_VAR");
    assert!(beta_pos < zebra_pos, "BETA_VAR should appear before ZEBRA_VAR");
}

#[test]
fn test_render_env_setup_block_special_characters_escaped() {
    let mut env = BTreeMap::new();
    env.insert("PATH_VAR".to_string(), "/some/path/value".to_string());

    let config = make_e2e_config_with_env(env);
    let result = render_env_setup_block(&config);

    assert!(
        result.contains("putenv('PATH_VAR=/some/path/value')"),
        "should preserve path"
    );
}

#[test]
fn test_render_run_tests_php_fails_loudly_when_extension_missing() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    let expected_failure_branch = "\
if (!file_exists($extPath)) {
    fwrite(STDERR, \"error: no sample_ext PHP extension build found.\\n\");
    foreach ($localExtCandidates as $candidate) {
        fwrite(STDERR, \"  looked for a local build at: $candidate\\n\");
    }
    $pieDisplay = $pieInstalledExtPath !== false ? $pieInstalledExtPath : '(unset)';
    fwrite(STDERR, \"  looked for PIE_INSTALLED_EXTENSION_PATH at: $pieDisplay\\n\");
    fwrite(STDERR, \"Build it locally with:\\n\");
    fwrite(STDERR, \"  cargo build --release -p sample-ext-php\\n\");
    exit(1);
}";
    assert!(
        result.contains(expected_failure_branch),
        "generated run_tests.php should fail loudly with both looked-up paths and a \
             build hint when no extension is found, got:\n{result}"
    );

    // The old silent-fallthrough guard (only re-exec when the extension exists) must be
    // gone -- the script now always exits above when $extPath is missing, so re-exec is
    // unconditional on ALEF_PHP_EXT_LOADED alone.
    assert!(
        !result.contains("if (file_exists($extPath) && !getenv('ALEF_PHP_EXT_LOADED')) {"),
        "should no longer silently defer to an ambient extension"
    );
}

/// The cdylib cargo emits is `lib` + the *package* name, and nothing else. The previous
/// fallback derived it from the extension name and appended its own `_php`, so an extension
/// named `sample_ext_php` was looked for as `libsample_ext_php_php` -- a path cargo never
/// writes, which no amount of building could satisfy.
#[test]
fn the_extension_library_name_comes_from_the_cargo_package_name() {
    let result = render_run_tests_php(
        "sample_ext_php",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    assert!(
        result.contains("libsample_ext_php'"),
        "library name must be lib + package name with hyphens underscored, got:\n{result}"
    );
    assert!(
        !result.contains("libsample_ext_php_php"),
        "the extension name must not have a second _php appended to it, got:\n{result}"
    );
}

/// A bare version keeps strict equality — the byte-for-byte shape every existing consumer
/// already generates.
#[test]
fn a_bare_version_still_compares_by_strict_equality() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    assert!(
        result.contains("if ($loadedVersion !== '1.2.3') {"),
        "a bare version must keep the strict-equality form, got:\n{result}"
    );
}

/// A comparison constraint has to go through `version_compare`. Comparing a version against a
/// constraint by string identity — which is what this used to emit — yields
/// `'3.12.3' !== '>=3.12.3'`, true for every build forever. The runner could not pass, and the
/// error named the extension rather than the configuration that made passing impossible.
#[test]
fn a_comparison_constraint_is_evaluated_rather_than_string_compared() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        ">=3.12.3",
        DependencyMode::Local,
    )
    .expect("a comparison constraint renders");

    assert!(
        result.contains("version_compare($loadedVersion, '3.12.3', '<')"),
        "a >= constraint must fail only when the loaded version is lower, got:\n{result}"
    );
    assert!(
        !result.contains("!== '>=3.12.3'"),
        "the constraint must never reach a string comparison, got:\n{result}"
    );
}

/// Anything alef cannot evaluate is refused at generation time. Emitting a check that cannot
/// pass is worse than refusing to emit one: it fails in the consumer's CI, blaming their
/// extension build, arbitrarily far from the configuration that caused it.
#[test]
fn a_constraint_that_cannot_be_evaluated_is_refused_at_generation_time() {
    let error = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "^3.12.3",
        DependencyMode::Local,
    )
    .expect_err("a caret constraint must be refused");

    assert!(
        error.to_string().contains("cannot \nevaluate") || error.to_string().contains("cannot evaluate"),
        "the refusal must say what it could not evaluate, got: {error:#}"
    );
}

/// A php extension crate excluded from the workspace builds into its own `target/`, never
/// the workspace-root one, so the harness must check both. Asserting only the workspace path
/// would pass for a member crate and silently fail for every excluded one.
#[test]
fn the_harness_checks_both_the_workspace_and_the_crate_local_target_dir() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    assert!(
        result.contains("$repoRoot . 'target/release/libsample_ext_php'"),
        "workspace-root target must still be a candidate, got:\n{result}"
    );
    assert!(
        result.contains("$repoRoot . 'crates/sample-ext-php/target/release/libsample_ext_php'"),
        "the crate-local target must be a candidate too, got:\n{result}"
    );
}

#[test]
fn test_render_run_tests_php_asserts_preflight_extension_version() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    let expected_version_branch = "\
$loadedVersion = $preflightStdout;
if ($loadedVersion !== '1.2.3') {
    fwrite(STDERR, \"error: loaded sample_ext extension version mismatch.\\n\");
    fwrite(STDERR, \"  extension at $extPath reports version: $loadedVersion\\n\");
    fwrite(STDERR, \"  expected version: 1.2.3\\n\");
    fwrite(STDERR, \"Rebuild it with:\\n\");
    fwrite(STDERR, \"  cargo build --release -p sample-ext-php\\n\");
    exit(1);
}";
    assert!(
        result.contains(expected_version_branch),
        "generated run_tests.php should assert the preflight-reported extension version \
             against the workspace version baked in at generation time, got:\n{result}"
    );
}

#[test]
fn test_render_run_tests_php_detects_already_loaded_warning() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    assert!(
        result.contains("stripos($preflightStderr, 'already loaded') !== false"),
        "generated run_tests.php should scan preflight stderr for an \"already loaded\" \
             startup warning instead of trusting the exit code alone, got:\n{result}"
    );
    assert!(
        result.contains("error: the sample_ext extension is already loaded from an ambient php.ini"),
        "the already-loaded failure message should name the extension and explain the \
             collision, got:\n{result}"
    );
}

#[test]
fn test_render_run_tests_php_builds_isolated_ini_before_loading_extension() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    assert!(
        result.contains("function alef_build_isolated_ini(string $php, string $extensionName): string {"),
        "generated run_tests.php should define a helper that filters the ambient php.ini \
             instead of trusting `-d extension=` alone to win, got:\n{result}"
    );
    assert!(
        result.contains("$isolatedIni = alef_build_isolated_ini($php, 'sample_ext');"),
        "the isolated ini must be built for this generated extension's own name, got:\n{result}"
    );
    assert!(
        result.contains("$phpConfigArgs = ['-n', '-c', $isolatedIni, '-d', 'extension=' . $extPath];"),
        "both PHPUnit invocation and the preflight check must run under `-n -c <isolated ini>` \
             plus an explicit `-d extension=` for the built extension, got:\n{result}"
    );
}

#[test]
fn test_render_run_tests_php_preflight_runs_unconditionally_before_phpunit() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("bare version renders");

    // Regression guard: a previous revision put the version/identity check after a
    // branch that unconditionally `passthru`'d into PHPUnit and then `exit`'d, so the
    // check could never execute. The preflight `proc_open` call (and the version check
    // that follows it) must appear strictly BEFORE the PHPUnit `passthru` invocation, and
    // no unconditional `exit($exitCode)` may sit between the start of the file and the
    // preflight version check.
    let preflight_pos = result
        .find("$process = proc_open(")
        .expect("preflight proc_open call must be present");
    let version_check_pos = result
        .find("if ($loadedVersion !== '1.2.3') {")
        .expect("preflight version check must be present");
    let passthru_pos = result
        .find("passthru(implode(' ', array_map('escapeshellarg', $cmd)), $exitCode);")
        .expect("PHPUnit passthru invocation must be present");

    assert!(
        preflight_pos < version_check_pos,
        "preflight identity check must run before the version comparison, got:\n{result}"
    );
    assert!(
        version_check_pos < passthru_pos,
        "the version check must run before PHPUnit is ever invoked, so it cannot be \
             stranded after an unconditional exit like the previous defect, got:\n{result}"
    );

    // The old re-exec gate (`ALEF_PHP_EXT_LOADED`) that caused the guard to be
    // unreachable must not reappear.
    assert!(
        !result.contains("ALEF_PHP_EXT_LOADED"),
        "the dead re-exec gate must not reappear -- the preflight check must be reached by \
             plain sequential execution of this single script, got:\n{result}"
    );
}

/// Regression for alef issue #368: install.sh and run_tests.php run as separate processes,
/// so an `export` in install.sh's shell never reaches run_tests.php's `getenv()`. The
/// default resolution must come from this process's own `ini_get('extension_dir')`, with
/// the env var kept only as an explicit override.
#[test]
fn run_tests_php_resolves_the_pie_path_from_extension_dir_by_default() {
    let result = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Registry,
    )
    .expect("bare version renders");

    assert!(
        result.contains("$registryExtDir = rtrim((string) ini_get('extension_dir'), '/');"),
        "the runner must recompute the registry extension dir from ini_get, got:\n{result}"
    );
    assert!(
        result.contains("$pieInstalledExtPath = $registryExtDir . '/sample_ext.so';"),
        "the recomputed path must use the .so suffix and this extension's own name, got:\n{result}"
    );
    assert!(
        !result.contains("$pieInstalledExtPath = $registryExtDir . '/sample_ext.dylib'"),
        "PHP extensions built by PIE are .so on every platform, Darwin included, got:\n{result}"
    );
    // The env var must still short-circuit the ini_get fallback when set, so an explicit
    // override is honored.
    let getenv_pos = result
        .find("$pieInstalledExtPath = getenv('PIE_INSTALLED_EXTENSION_PATH');")
        .expect("getenv call present");
    let fallback_pos = result
        .find("if (!$pieInstalledExtPath && $resolvePieFromExtensionDir) {")
        .expect("fallback guard present");
    assert!(
        getenv_pos < fallback_pos,
        "the env var must be read before the ini_get fallback runs, got:\n{result}"
    );
}

/// A local-mode harness tests this checkout's cargo build. It must not default to a copy of
/// the extension that a PIE/PECL install of an earlier release left in `extension_dir`,
/// which `alef e2e generate` at 0.93.0 did: on a machine carrying such an install every e2e
/// run aborted on the version-mismatch guard, and without the guard would have tested stale
/// code. The env var stays an explicit override in both modes.
#[test]
fn run_tests_php_only_defaults_to_extension_dir_in_registry_mode() {
    let local = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Local,
    )
    .expect("renders");
    assert!(
        local.contains("$resolvePieFromExtensionDir = false;"),
        "local mode must not read extension_dir by default, got:\n{local}"
    );
    assert!(
        local.contains("if (!$pieInstalledExtPath && $resolvePieFromExtensionDir) {"),
        "the env var override must remain honoured, got:\n{local}"
    );
    let registry = render_run_tests_php(
        "sample_ext",
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Registry,
    )
    .expect("renders");
    assert!(
        registry.contains("$resolvePieFromExtensionDir = true;"),
        "registry mode keeps the #368 default, got:\n{registry}"
    );
}

/// Executable proof of the fix in [`run_tests_php_resolves_the_pie_path_from_extension_dir_by_default`]:
/// the generated resolution snippet, run through real PHP as its own process with
/// `PIE_INSTALLED_EXTENSION_PATH` unset and no local build artifacts on disk, still
/// resolves the PIE-installed `.so` -- entirely from `ini_get('extension_dir')` -- exactly
/// as it would immediately after `bash install.sh` ran in a separate shell.
///
/// Unix-only, like [`super::install_sh_execution_tests`]: this proves the same PIE/`install.sh`
/// registry-mode flow that module already restricts to `#[cfg(all(test, unix))]`, and for the
/// same reason -- PIE has no Windows support, `install.sh` is bash, and the `.so` suffix this
/// snippet resolves is a Unix/Darwin-only artifact name (a real Windows PECL build produces
/// `php_<name>.dll`, never `.so`). Running the whole scenario through an arbitrary Windows
/// `php.exe` proves nothing about a code path no real Windows user ever reaches: this specific
/// harness failed on windows-latest CI (asserting the untouched local-build fallback, not a
/// mangled resolved path), which is consistent with the resolution branch never firing on that
/// platform rather than with any defect in the generator's own path handling. ~keep
#[cfg(unix)]
#[test]
#[allow(clippy::print_stderr)] // narrow: reports a toolchain skip on a developer machine without PHP ~keep
fn run_tests_php_resolution_snippet_finds_the_extension_across_processes() {
    let php = match std::process::Command::new("php").arg("--version").output() {
        Ok(output) if output.status.success() => "php",
        _ => {
            eprintln!("skipping: no `php` interpreter on PATH");
            return;
        }
    };

    let root = tempfile::tempdir().expect("tempdir");
    let ext_dir = root.path().join("ext_dir");
    std::fs::create_dir_all(&ext_dir).expect("create fake extension_dir");
    let extension_name = "sample_ext";
    std::fs::write(ext_dir.join(format!("{extension_name}.so")), b"fake extension").expect("write fake .so");

    let generated = render_run_tests_php(
        extension_name,
        "sample-ext-php",
        "crates/sample-ext-php",
        "1.2.3",
        DependencyMode::Registry,
    )
    .expect("bare version renders");

    // Extract exactly the resolution snippet under test from the real generated file,
    // so this proves the shipped code resolves correctly, not a hand-copied stand-in.
    let start = generated
        .find("// Check for a PIE-installed extension path")
        .expect("resolution comment present");
    let end = generated
        .find("// Neither a local release build")
        .expect("end-of-block marker present");
    let snippet = &generated[start..end];
    assert!(
        snippet.contains("getenv('PIE_INSTALLED_EXTENSION_PATH')"),
        "extracted snippet must be the resolution block, got:\n{snippet}"
    );

    // No local build artifact exists anywhere the harness would look, so a pass here can
    // only be explained by the PIE/ini_get fallback, not the local-candidate search.
    let script = format!(
        "<?php\n$extPath = '{missing}';\n{snippet}\necho $extPath;",
        missing = root.path().join("target/release/nonexistent").display(),
    );
    let script_path = root.path().join("resolve.php");
    std::fs::write(&script_path, script).expect("write resolution script");

    // `-n` skips the ambient php.ini/conf.d entirely: this machine's own PHP install may
    // declare unrelated real extensions there, which would otherwise fail to load from the
    // overridden `extension_dir` below and spew startup warnings onto stdout, polluting the
    // very output this assertion reads. `-d extension_dir=...` still applies under `-n`.
    let output = std::process::Command::new(php)
        .arg("-n")
        .arg("-d")
        .arg(format!("extension_dir={}", ext_dir.display()))
        .arg(&script_path)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .output()
        .expect("run resolution snippet under real PHP");

    assert!(
        output.status.success(),
        "resolution snippet must run cleanly, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let resolved = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        resolved,
        format!("{}/{extension_name}.so", ext_dir.display()),
        "the snippet must resolve the PIE-installed .so via ini_get('extension_dir') alone, \
             with PIE_INSTALLED_EXTENSION_PATH unset and no local build artifact present"
    );
}
