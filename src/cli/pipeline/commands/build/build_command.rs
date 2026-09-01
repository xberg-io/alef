//! Where each language's binding crate lives, and the shell command that builds it.
//!
//! Split out of `build.rs` so the orchestration half (scheduling, readiness, outcome reporting)
//! and the per-language command construction half stay separately reviewable. ~keep

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::template_versions as tv;
use std::path::Path;

/// Resolve the crate directory from the output config path.
/// Output paths like `crates/sample-markdown-node/src/` → `crates/sample-markdown-node`.
pub(super) fn resolve_crate_dir(output_path: &Path) -> &Path {
    if output_path.file_name().is_some_and(|n| n == "src") {
        output_path.parent().unwrap_or(output_path)
    } else {
        output_path
    }
}

/// Get the output path for a language from config.
pub(super) fn output_path_for(lang: Language, config: &ResolvedCrateConfig) -> Option<&Path> {
    match lang {
        Language::Python => config.explicit_output.python.as_deref(),
        Language::Node => config.explicit_output.node.as_deref(),
        Language::Ruby => config.explicit_output.ruby.as_deref(),
        Language::Php => config.explicit_output.php.as_deref(),
        Language::Ffi => config.explicit_output.ffi.as_deref(),
        Language::Go => config.explicit_output.go.as_deref(),
        Language::Java => config.explicit_output.java.as_deref(),
        Language::Csharp => config.explicit_output.csharp.as_deref(),
        Language::Kotlin => config.explicit_output.kotlin.as_deref(),
        Language::KotlinAndroid => config.explicit_output.kotlin_android.as_deref(),
        Language::Wasm => config.explicit_output.wasm.as_deref(),
        Language::Elixir => config.explicit_output.elixir.as_deref(),
        Language::R => config.explicit_output.r.as_deref(),
        Language::Rust | Language::C | Language::Jni => None,
        Language::Swift | Language::Dart | Language::Gleam | Language::Zig => None,
    }
}

/// Generate the shell command to build a specific language.
pub(super) fn build_command_for(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    release: bool,
) -> String {
    let release_flag = if release { " --release" } else { "" };

    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .and_then(|p| p.to_str())
        .unwrap_or("");

    match bc.tool {
        "maturin" => {
            // `crate_dir` is empty whenever `[crates.output] python` is not set explicitly (the
            // common case) — the same dangling `--manifest-path /Cargo.toml` the `napi` arm below
            // was fixed for. Resolving it is load-bearing here and not merely tidy: the
            // `--features` flag is derived by reading the manifest, so an unresolvable path would
            // make the feature check examine nothing and silently emit no flag at all. ~keep
            let py_crate_dir = if crate_dir.is_empty() {
                // `core_crate_dir`, not `config.name`: it is the directory name
                // `scaffold::languages::python` derives the manifest path from, and the two must
                // name one file for the feature probe below to read the manifest that exists.
                let core_crate_dir = config.core_crate_dir();
                crate::core::config::resolve_helpers::default_binding_crate_root(&core_crate_dir, "python")
                    .unwrap_or_else(|| format!("crates/{core_crate_dir}-py"))
            } else {
                crate_dir.to_string()
            };
            // The generated crate declares `extension-module` outside its default features, and
            // the generated `pyproject.toml` requests the pyo3 features it turns on — but maturin
            // reads that `pyproject.toml` only when it sits beside the manifest, which it never
            // does here (it lives in the Python package directory). Without an explicit
            // `--features`, pyo3 falls back to linking libpython and drops abi3 on every
            // platform; on Darwin that still links and loads, so a green build proves nothing
            // about the extension surface it produced. Asks the manifest which feature it
            // declares rather than restating the name. ~keep
            let manifest = Path::new(&py_crate_dir).join("Cargo.toml");
            let features_flag = crate::core::config::python_build::extension_module_feature_flag(&manifest);
            // Quoted only around `py_crate_dir` itself: `sh` concatenates a quoted word with its
            // unquoted `/Cargo.toml` suffix into one argument, the same shape as the `ffi` arm's
            // `--manifest-path` (see `ffi_manifest_path_reaches_cargo_as_one_unquoted_path`). ~keep
            let py_crate_dir = crate::core::config::shell::quote_word(&py_crate_dir);
            let command =
                format!("maturin develop --manifest-path {py_crate_dir}/Cargo.toml{features_flag}{release_flag}");
            crate::core::config::python_build::run_through_python_package_manager(command, &config.tools)
        }
        "napi" => {
            // `crate_dir` is empty whenever `[crates.output] node` is not set explicitly (the
            // common case) — fall back to the same default formula `package_dir` already uses
            // for scaffolding, matching the `wasm-pack` arm below. Without this, the common
            // no-override case sent napi a dangling `--manifest-path /Cargo.toml -o `, which
            // resolves against the repo root instead of the generated crate. ~keep
            let node_crate_dir = if crate_dir.is_empty() {
                config.package_dir(lang)
            } else {
                crate_dir.to_string()
            };
            // napi-rs resolves the package name it bakes into the generated JS loader (and,
            // with `--platform`, the optional-dependency package names for every target) from
            // the `package.json` it reads — which defaults to `<cwd>/package.json`, not a path
            // derived from `--manifest-path`/`-o`. alef always invokes napi from the repo root,
            // so without `--package-json-path` napi silently reads the *workspace* root's
            // `package.json` in any repo that has one, and bakes that package's name into the
            // loader instead of the binding crate's own. `--package-json-path` names the correct
            // file directly, so the fix holds regardless of `--cwd` and needs no `cd` into the
            // crate directory (which would also require rewriting `--manifest-path`/`-o` to be
            // relative to it). See alef#368. ~keep
            // Each occurrence is quoted independently rather than once as a shared variable: the
            // three flags place `node_crate_dir` in three separate shell words
            // (`--manifest-path 'dir'/Cargo.toml`, `-o 'dir'`, `--package-json-path
            // 'dir'/package.json`), and `sh` concatenates a quoted word with an unquoted suffix
            // into one argument, same shape as the `ffi`/`maturin` manifest-path arms. ~keep
            let node_crate_dir = crate::core::config::shell::quote_word(&node_crate_dir);
            format!(
                "npx --yes -p @napi-rs/cli@{} napi build --platform --no-js --manifest-path {}/Cargo.toml -o {} \
                 --package-json-path {}/package.json --dts {}{}",
                tv::npm::NAPI_RS_CLI_CRATE,
                node_crate_dir,
                node_crate_dir,
                node_crate_dir,
                tv::npm::NAPI_AUTO_DTS_FILENAME,
                release_flag
            )
        }
        "wasm-pack" => {
            let profile = if release { "--release" } else { "--dev" };
            // `crate_dir` is empty whenever `[crates.output] wasm` is not set explicitly
            // (the common case) — fall back to the same default formula `package_dir`
            // already uses for scaffolding, matching the `gradle`/`swift`/`zig` arms below.
            let wasm_crate_dir = if crate_dir.is_empty() {
                config.package_dir(lang)
            } else {
                crate_dir.to_string()
            };
            // Build every configured target into `pkg/<target>`, byte-for-byte matching the
            // `build:wasm:<target>` scripts `scaffold::languages::wasm` writes into the crate's
            // own `package.json` (and which `publish::package::wasm` re-runs via `npm run
            // build:all`). Nothing consumes a bare `pkg/`: the scaffolded manifest resolves
            // `main`/`types` to `pkg/<nodejs>/…` and `module` to `pkg/<web>/…`, and `alef e2e`
            // depends on `pkg/nodejs` specifically (`ResolvedCrateConfig::wasm_crate_path`).
            // The previous `--target web` with wasm-pack's default out-dir wrote a bare `pkg/`
            // that no manifest, publish step or e2e suite reads, while never producing the
            // `pkg/nodejs` e2e needs — which is why consumers hand-rolled an extra build step.
            // Driving off `wasm_targets()` rather than a hardcoded pair also honours a crate
            // that narrows `[crates.wasm] targets` to keep its published package small. `cd`
            // into the crate dir instead of passing a positional `<path>`, since wasm-pack
            // resolves `--out-dir` against the process cwd, not against that argument. ~keep
            let targets = config.wasm_targets();
            if targets.is_empty() {
                // `scaffold::languages::wasm` rejects an empty target list, but that runs only
                // when scaffolding; reaching here with `targets = []` would otherwise emit a
                // dangling `cd <dir> && ` that the shell reports as a syntax error naming
                // neither alef nor the setting at fault. ~keep
                return format!(
                    "echo 'alef: [crates.wasm] targets is empty for {lang}; list at least one \
                     wasm-pack target (web, bundler, nodejs, deno)' >&2 && false"
                );
            }
            let builds = targets
                .iter()
                .map(|target| format!("wasm-pack build {profile} --target {target} --out-dir pkg/{target}"))
                .collect::<Vec<_>>()
                .join(" && ");
            // `wasm_crate_dir` is a consumer-configured `[crates.output] wasm` path (or the
            // `package_dir` fallback), reaching `sh -c` as the `cd` target -- quote it the same
            // way the `gradle`/`dotnet`/`go` arms below do. ~keep
            let wasm_crate_dir = crate::core::config::shell::quote_word(&wasm_crate_dir);
            format!("cd {wasm_crate_dir} && {builds}")
        }
        "cargo" => {
            // `-p <name>-ffi` is a workspace package spec: it resolves only if the emitted crate
            // is a member of the workspace cargo is invoked from. The generated FFI crate is
            // standalone whenever the consumer does not list it under `[workspace] members`, and
            // cargo then rejects the spec outright rather than falling back to the path — which
            // is what broke the generated-output gate's own fixture. `default_binding_crate_root`
            // is the same formula `OutputTemplate::resolve` uses for the default output path, so
            // this and the tree `generate` writes cannot name two different crate roots. Building
            // by manifest path is correct for a workspace member too, so no consumer loses the
            // workspace-aware behaviour. ~keep
            if crate_dir.is_empty()
                && lang == Language::Ffi
                && let Some(root) =
                    crate::core::config::resolve_helpers::default_binding_crate_root(&config.name, "ffi")
            {
                // `root` is derived from `config.name` (`[[crates]] name`), consumer-configured
                // and reaching `sh -c` here -- quoted-prefix + unquoted `/Cargo.toml` suffix
                // concatenates into one argument, the same shape as the `ffi` arm's own default
                // in `build_defaults.rs`. ~keep
                let root = crate::core::config::shell::quote_word(&root);
                return format!("cargo build --manifest-path {root}/Cargo.toml{release_flag}");
            }
            if crate_dir.is_empty() && !bc.crate_suffix.is_empty() {
                return format!("cargo build -p {}{}{}", config.name, bc.crate_suffix, release_flag);
            }
            let native_dir = Path::new(crate_dir).join("native");
            let native_manifest = native_dir.join("Cargo.toml");
            if native_manifest.exists() {
                // `native_dir` is derived from `crate_dir`, itself a consumer-configured
                // `[crates.output] ffi` path, reaching `sh -c` as the `cd` target. ~keep
                let dir = crate::core::config::shell::quote_word(&native_dir.display().to_string());
                format!("cd {dir} && cargo build{release_flag}")
            } else if let Some(standalone) = {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut found: Option<std::path::PathBuf> = None;
                for _ in 0..3 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[workspace]")
                        {
                            found = Some(p.clone());
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                found
            } {
                // `standalone` walks up from `crate_dir` (a consumer-configured `[crates.output]
                // ffi` path) looking for the enclosing workspace root -- still consumer-derived,
                // still reaching `sh -c` as the `cd` target. ~keep
                let dir = crate::core::config::shell::quote_word(&standalone.display().to_string());
                format!("cd {dir} && cargo build{release_flag}")
            } else {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut package_name: Option<String> = None;
                let mut package_dir: Option<std::path::PathBuf> = None;
                for _ in 0..4 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[package]")
                        {
                            for line in contents.lines() {
                                let trimmed = line.trim();
                                if let Some(rest) = trimmed.strip_prefix("name") {
                                    let rest = rest.trim_start_matches([' ', '=']).trim();
                                    let rest = rest.trim_matches(['"', '\'']);
                                    if !rest.is_empty() {
                                        package_name = Some(rest.to_string());
                                        package_dir = Some(p.clone());
                                        break;
                                    }
                                }
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                let is_excluded_from_workspace = if let Some(pdir) = &package_dir {
                    let mut q = pdir.clone();
                    let mut excluded = false;
                    while q.pop() {
                        let manifest = q.join("Cargo.toml");
                        if manifest.exists()
                            && let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[workspace]")
                        {
                            let rel = pdir.strip_prefix(&q).unwrap_or(pdir).to_string_lossy().into_owned();
                            let rel_norm = rel.replace('\\', "/");
                            excluded = contents.lines().map(|l| l.trim()).any(|l| {
                                l.contains(&format!("\"{rel_norm}\"")) && {
                                    let needle = format!("\"{rel_norm}\"");
                                    let exclude_section = contents.split("exclude").nth(1).unwrap_or("");
                                    let members_section = contents.split("members").nth(1).unwrap_or("");
                                    let in_exclude = exclude_section.contains(&needle);
                                    let in_members =
                                        members_section.contains(&needle) && !exclude_section.contains(&needle);
                                    in_exclude && !in_members
                                }
                            });
                            break;
                        }
                    }
                    excluded
                } else {
                    false
                };
                if is_excluded_from_workspace {
                    if let Some(pdir) = package_dir {
                        // `pdir` is derived from `crate_dir`, a consumer-configured
                        // `[crates.output] ffi` path, reaching `sh -c` as the `cd` target. ~keep
                        let dir = crate::core::config::shell::quote_word(&pdir.display().to_string());
                        format!("cd {dir} && cargo build{release_flag}")
                    } else {
                        let dir = crate::core::config::shell::quote_word(crate_dir);
                        format!("cd {dir} && cargo build{release_flag}")
                    }
                } else {
                    let crate_name = package_name.unwrap_or_else(|| {
                        Path::new(crate_dir)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(crate_dir)
                            .to_string()
                    });
                    format!("cargo build -p {crate_name}{release_flag}")
                }
            }
        }
        "mix" => {
            let dir = config
                .explicit_output
                .elixir
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/elixir");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("mix.exs").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            // Consumer-configured `[crates.output] elixir` path (or a `mix.exs` directory found
            // beside it on disk), reaching `sh -c` as the `cd` target. ~keep
            let build_dir = crate::core::config::shell::quote_word(&build_dir);
            format!("cd {build_dir} && mix compile")
        }
        "mvn" => {
            let dir = config
                .explicit_output
                .java
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/java");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("pom.xml").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            // Consumer-configured `[crates.output] java` path (or a `pom.xml` directory found
            // beside it on disk), reaching `sh -c` as the `cd` target. ~keep
            let build_dir = crate::core::config::shell::quote_word(&build_dir);
            format!("cd {build_dir} && mvn package -DskipTests --batch-mode --no-transfer-progress")
        }
        "dotnet" => {
            let dir = config
                .explicit_output
                .csharp
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/csharp");
            let scan_for_csproj = |start: &std::path::Path| -> Option<String> {
                if start
                    .read_dir()
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
                    })
                    .unwrap_or(false)
                {
                    return Some(start.to_string_lossy().to_string());
                }
                start.read_dir().ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .find(|e| {
                            e.path().is_dir()
                                && e.path().read_dir().ok().is_some_and(|sub| {
                                    sub.filter_map(|s| s.ok())
                                        .any(|s| s.path().extension().is_some_and(|ext| ext == "csproj"))
                                })
                        })
                        .map(|e| e.path().to_string_lossy().to_string())
                })
            };
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                let mut found = scan_for_csproj(&p);
                while found.is_none() && p.pop() {
                    found = scan_for_csproj(&p);
                }
                found.unwrap_or_else(|| dir.to_string())
            };
            let dotnet_config = if release { "Release" } else { "Debug" };
            // `build_dir` is derived from a consumer-configured `[crates.output] csharp` path
            // (or a `.csproj` directory found beside it on disk), reaching `sh -c` as the `cd`
            // target -- quote it the same way the `gradle` arm below does. ~keep
            let build_dir = crate::core::config::shell::quote_word(&build_dir);
            format!("cd {build_dir} && dotnet build --configuration {dotnet_config} --verbosity quiet")
        }
        "go" => {
            let dir = config
                .explicit_output
                .go
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/go");
            // Consumer-configured `[crates.output] go` path, reaching `sh -c` as the `cd`
            // target. ~keep
            let dir = crate::core::config::shell::quote_word(dir);
            format!("cd {dir} && go build ./...")
        }
        "gradle" => {
            let source_dir = if crate_dir.is_empty() {
                config.package_dir(lang)
            } else {
                crate_dir.to_string()
            };
            // `crate_dir`/`package_dir` is a *source* output path (e.g. a Kotlin
            // package-namespace directory `.kt` files get written into) — it need not
            // be the gradle project root gradle itself will accept. `gradle <task>`
            // walks up from the invoked directory looking only for a settings file and
            // treats that file's directory as the build root; a directory that is
            // merely nested under that root but not itself a registered project is
            // rejected ("Project directory '...' is not part of the build defined by
            // settings file '...'") — that rejection, observed against a real consumer
            // whose `[crates.output] kotlin_android` pointed at a deep namespace source
            // dir, is this arm's bug. So we search for `settings.gradle.kts`/
            // `settings.gradle` first, walking the full ancestor chain, mirroring
            // gradle's own root-detection precedence: a settings file is authoritative
            // over a build file, because a `build.gradle(.kts)` alone only marks *a*
            // project (possibly a subproject with no invocable root of its own). Only
            // if no settings file is found anywhere upward do we fall back to
            // `build.gradle.kts`/`build.gradle` as a lower-confidence signal for a
            // settings-less single-module project; finding neither falls back to
            // `source_dir` unchanged, matching every other backend arm in this
            // function (mix/mvn/dotnet) rather than walking to the filesystem root or
            // panicking.
            //
            // Deliberately NOT sharing a helper with the "dotnet" arm's `scan_for_csproj`
            // above: that is the only other precedent for this shape, so per this repo's
            // "extract on the third repetition" convention the two stay separate — kept
            // structurally parallel (closure + walk-up loop) instead. ~keep
            let find_marker_dir = |start: &str, markers: &[&str]| -> Option<String> {
                let mut p = std::path::PathBuf::from(start);
                loop {
                    if markers.iter().any(|marker| p.join(marker).exists()) {
                        return Some(p.to_string_lossy().into_owned());
                    }
                    if !p.pop() {
                        return None;
                    }
                }
            };
            let build_dir = find_marker_dir(&source_dir, &["settings.gradle.kts", "settings.gradle"])
                .or_else(|| find_marker_dir(&source_dir, &["build.gradle.kts", "build.gradle"]))
                .unwrap_or(source_dir);
            // The task itself must come from `Language`, not from this arm's shared `"gradle"`
            // tool string: `Kotlin` and `KotlinAndroid` both dispatch here, but only
            // `gradle_build_task` can tell them apart. Asking `build_defaults`'s helper keeps
            // this call site and `default_build_config`'s own Kotlin/KotlinAndroid arms deriving
            // the same answer instead of two independent ones (xberg-io/alef#259). ~keep
            let task = crate::core::config::build_defaults::gradle_build_task(lang, release);
            // Quoted for the same reason `build_defaults` quotes its own `cd {output_dir}`: this
            // string reaches `sh -c`, and `build_dir` is derived from a consumer-configured
            // `[crates.output]` path (or from a directory name found beside it on disk). It is
            // also what keeps this arm byte-identical to the built-in default the two call sites
            // are cross-checked against in `kotlin_android_gradle_arm_matches_build_defaults_default`
            // — an unquoted answer here would be a *different command* from the one alef ships as
            // the default, which is exactly the divergence xberg-io/alef#259 was about. ~keep
            let build_dir = crate::core::config::shell::quote_word(&build_dir);
            format!("cd {build_dir} && gradle {task}")
        }
        "swift" => {
            // `package_dir` is a consumer-configured `[crates.output] swift` path (or its
            // default), reaching `sh -c` as `--package-path`'s value -- quote it the same way
            // `default_build_config`'s own Swift arm does, so the two producers stay
            // byte-identical. ~keep
            let package_dir = crate::core::config::shell::quote_word(&config.package_dir(lang));
            let configuration = if release { " --configuration release" } else { "" };
            format!("swift build --package-path {package_dir}{configuration}")
        }
        "zig" => {
            let package_dir = crate::core::config::shell::quote_word(&config.package_dir(lang));
            let release_flag = if release { " --release=fast" } else { "" };
            format!("cd {package_dir} && zig build{release_flag}")
        }
        "gleam" => {
            let package_dir = crate::core::config::shell::quote_word(&config.package_dir(lang));
            format!("cd {package_dir} && gleam build")
        }
        // Every backend registers a `tool` here from the fixed set matched above. Reaching this
        // arm means a backend's `BuildConfig.tool` genuinely has no known default — fail loudly
        // and name the missing tool rather than silently substituting a bare `false`, which
        // previously reported as an inscrutable "Command failed: false". 0.82.0 removed
        // `[build_commands.<lang>]` from `alef.toml`, so there is no config-side fix to point at
        // any more — this is now always an alef bug, not a missing consumer override. ~keep
        _ => format!(
            "echo 'alef: no default build command for tool \"{}\" (language: {lang}); this is a \
             gap in alef'\\''s own built-in defaults -- alef.toml cannot configure it, please file \
             an issue at https://github.com/xberg-io/alef/issues' >&2 && false",
            bc.tool
        ),
    }
}
