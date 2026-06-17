use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::capitalize_first, scaffold::cargo_package_header, scaffold::core_dep_features,
    scaffold::detect_workspace_inheritance, scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

pub(crate) fn scaffold_elixir_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let nif_name = format!("{app_name}_nif");
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Elixir);
    let native_crate_dir = format!("{pkg_dir}/native/{nif_name}");
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(&nif_name, version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Elixir);
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    // Trait bridges generate async_trait impls and a tokio::sync::oneshot-based
    // reply channel, so async-trait and tokio are required whenever there are
    // active Elixir trait bridges even if no API functions are async.
    let has_trait_bridges = config
        .trait_bridges
        .iter()
        .any(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"));
    // Streaming adapters generate `use futures_util::StreamExt` plus a
    // `futures_util::stream::BoxStream` field on the per-adapter handle struct,
    // so the scaffold must add `futures-util` whenever a streaming adapter is
    // declared on this crate.
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    // ahash is needed when any function takes an AHashMap<Cow, _> param — the generated
    // NIF emits a `let __<name>_ahash: ahash::AHashMap<...>` pre-call binding.
    let needs_ahash = api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let lib_path_line = if let Some(elixir_out) = config.explicit_output.elixir.as_ref() {
        let output_dir = elixir_out.to_string_lossy();
        if output_dir.contains("/native/") {
            String::new()
        } else {
            let native_depth = std::path::Path::new(&native_crate_dir).components().count();
            let output_path = output_dir.trim_end_matches('/');
            let lib_path = format!(
                "{}{}{}",
                "../".repeat(native_depth),
                output_path.trim_start_matches('/'),
                "/lib.rs"
            );
            format!("path = \"{lib_path}\"\n")
        }
    } else {
        String::new()
    };

    // Collect all [dependencies] entries then sort alphabetically so the emitted
    // Cargo.toml is cargo-sort canonical without a post-processing step.
    let features_str = core_dep_features(config, Language::Elixir);
    let mut dep_lines: Vec<String> = vec![
        crate::scaffold::render_core_dep(
            &config.name,
            &format!("../../../../crates/{core_crate_dir}"),
            &features_str,
            version,
        ),
        format!("rustler = \"{}\"", tv::cargo::RUSTLER),
        "serde = { version = \"1\", features = [\"derive\"] }".to_owned(),
        "serde_json = \"1\"".to_owned(),
    ];
    if needs_ahash {
        dep_lines.push("ahash = \"0.8\"".to_owned());
    }
    if has_trait_bridges {
        dep_lines.push(format!("async-trait = \"{}\"", tv::cargo::ASYNC_TRAIT));
    }
    if has_async || has_trait_bridges || has_streaming {
        dep_lines.push("tokio = { version = \"1\", features = [\"rt-multi-thread\", \"sync\"] }".to_owned());
    }
    if has_streaming && !dep_lines.iter().any(|l| l.starts_with("futures-util")) {
        dep_lines.push("futures-util = \"0.3\"".to_owned());
    }
    for line in extra_deps.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !dep_lines
                .iter()
                .any(|l| l.starts_with(trimmed.split('=').next().unwrap_or("")))
        {
            dep_lines.push(trimmed.to_owned());
        }
    }
    // Pin transitive crates that conflict in the brotli 8.0.x family. Adding them as
    // direct dependencies forces cargo's resolver to pick the named versions for the
    // whole dep tree; a `[patch.crates-io]` entry with only `version` is a no-op
    // (cargo errors with "patch points to the same source"), so direct deps are the
    // correct mechanism. The NIF wrapper does not reference these symbols — they are
    // version-pin markers only, hence the cargo-machete ignore additions below.
    dep_lines.push("alloc-no-stdlib = \"=2.0.4\"".to_owned());
    dep_lines.push("alloc-stdlib = \"=0.2.2\"".to_owned());
    dep_lines.push("brotli-decompressor = \"=5.0.1\"".to_owned());
    dep_lines.sort();
    let deps_section = dep_lines.join("\n");

    // Build the cargo-machete ignored list. When `tokio` is included in
    // dependencies (for async, trait bridges, or streaming), mark it as ignored
    // since the NIF wrapper may not directly reference it. Same for `async-trait`
    // (included for trait bridges) and `futures-util` (included for streaming).
    // `ahash` is added when any function parameter uses AHashMap<Cow, _>, but the
    // NIF never directly uses ahash—it's used only in the Rust core for type
    // field marshalling.
    let mut machete_ignored: Vec<&str> = Vec::new();
    if has_async || has_trait_bridges || has_streaming {
        machete_ignored.push("tokio");
    }
    if has_trait_bridges {
        machete_ignored.push("async-trait");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    if needs_ahash {
        machete_ignored.push("ahash");
    }
    // Version-pin direct deps are never referenced from NIF code; mark ignored so
    // cargo-machete does not flag them.
    machete_ignored.push("alloc-no-stdlib");
    machete_ignored.push("alloc-stdlib");
    machete_ignored.push("brotli-decompressor");
    // cargo-sort places `[package.metadata.*]` immediately after `[package]`,
    // before `[workspace]`, `[lib]`, `[features]`, `[dependencies]`.
    let machete_section = if machete_ignored.is_empty() {
        String::new()
    } else {
        let ignored_list = machete_ignored
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[package.metadata.cargo-machete]\nignored = [{ignored_list}]\n\n")
    };

    // Collect every upstream feature name referenced via `#[cfg(feature = "X")]` in the
    // generated enum From-impl arms so that rustc's `unexpected_cfgs` lint accepts them
    // under `-D warnings`. Mirrors the dart 0.25.9 + swift 0.25.11 check-cfg
    // allow-lists for backends that don't (yet) implement Option B feature forwarding.
    let referenced_features = crate::codegen::cfg::collect_cfg_features(api);

    // Hard-code the canonical core features that the rustler backend uses for
    // conditional method generation (config, download, serde) even if cfg-only
    // detection returns empty. The rustler codegen emits #[cfg(feature = "X")]
    // guards post-hoc during NIF function generation, not as IR-level features,
    // so collect_cfg_features(api) returns empty. Ensure these are always present
    // in the [features] block so the NIF crate can forward them to the core crate.
    let mut always_features: std::collections::BTreeSet<String> =
        ["download", "serde", "config"].iter().map(|s| s.to_string()).collect();
    always_features.extend(referenced_features.clone());

    // Emit a [features] block with `default = [...]` and forwarding entries like
    // `download = ["<core-pkg>/download"]` so the rustler NIF crate can forward
    // features to the core crate. Without this, #[cfg(feature = "X")] arms fail
    // when the binding crate's Cargo.toml doesn't declare those features.
    // Mirrors the ruby/swift/dart/napi/php pattern (see commit 3b8aa6fc9 for ruby).
    let features_table = {
        let mut lines: Vec<String> = Vec::with_capacity(always_features.len() + 1);
        let default_list: Vec<String> = always_features.iter().map(|name| format!("\"{name}\"")).collect();
        lines.push(format!("default = [{}]", default_list.join(", ")));
        for name in &always_features {
            lines.push(format!(
                r#"{name} = ["{core_dep_key}/{name}"]"#,
                core_dep_key = config.name
            ));
        }
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    // Emit `[lints.rust]` at end of file (after `[dependencies]`) to match
    // cargo-sort's canonical ordering. Emitting it before `[dependencies]`
    // matches the template but cargo-sort (run by consumer repos' prek) moves
    // it to the bottom, producing a perpetual diff against the committed file
    // and failing strict version-sync checks in CI.
    let check_cfg_block = {
        let csv = always_features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "\n[lints.rust]\nunexpected_cfgs = {{ level = \"warn\", check-cfg = ['cfg(feature, values({csv}))'] }}\n"
        )
    };

    let content = format!(
        r#"{pkg_header}

{machete_section}[workspace]

[lib]
name = "{nif_name}"
{lib_path_line}
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_section}{check_cfg_block}"#,
        pkg_header = pkg_header,
        machete_section = machete_section,
        nif_name = nif_name,
        lib_path_line = lib_path_line,
        features_table = features_table,
        check_cfg_block = check_cfg_block,
        deps_section = deps_section,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("{native_crate_dir}/Cargo.toml")),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_elixir(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let nif_name = format!("{app_name}_nif");
    let version = &api.version;
    let pkg_dir = config.package_dir(Language::Elixir);
    let nif_targets = elixir_nif_targets(config).join(" ");

    // Jason is always required for Elixir bindings because generated data-class
    // serialization (pack_config, code_chunk, etc.) uses Jason.encode! / Jason.decode!,
    // in addition to any visitor bridges that may use it.
    let jason_dep = format!("\n      {{:jason, \"{jason}\"}},", jason = tv::hex::JASON);

    // Determine if the generated Elixir source files live outside the default `lib/`
    // subdirectory. If so, emit an `elixirc_paths` entry so Mix can find them.
    // The same external path is also added to the Hex `files:` list below, since
    // `mix hex.publish` verifies every entry exists on disk.
    let external_elixir_src: Option<String> = config.explicit_output.elixir.as_ref().and_then(|elixir_out| {
        let elixir_out_str = elixir_out.to_string_lossy();
        let expected_lib = format!("{pkg_dir}/lib");
        if elixir_out_str.starts_with(&expected_lib) {
            return None;
        }
        let pkg = std::path::Path::new(&pkg_dir);
        let out = std::path::Path::new(elixir_out_str.trim_end_matches('/'));
        let pkg_depth = pkg.components().count();
        let out_path = out.display().to_string();
        Some(format!(
            "{}{}",
            "../".repeat(pkg_depth),
            out_path.trim_start_matches('/')
        ))
    });

    let elixirc_paths_line = match external_elixir_src.as_deref() {
        Some(relative) => format!("\n      elixirc_paths: [\"lib\", Path.expand(\"{relative}\", __DIR__)],"),
        None => String::new(),
    };

    // Format rustler_crates in multi-line pre-formatted shape to match mix format's output.
    // Use a list literal for targets so mix format can wrap each target on its own line.
    // This ensures idempotent formatting regardless of the number of targets or library name length.
    let nif_targets_list: Vec<&str> = nif_targets.split_whitespace().collect();
    let last_idx = nif_targets_list.len().saturating_sub(1);
    let targets_lines = nif_targets_list
        .iter()
        .enumerate()
        .map(|(idx, target)| {
            if idx == last_idx {
                format!("            \"{target}\"")
            } else {
                format!("            \"{target}\",")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rustler_crates_block = format!(
        "rustler_crates: [\n        {nif_atom}: [\n          mode: :release,\n          targets: [\n{targets_lines}\n          ]\n        ]\n      ],",
        nif_atom = format_args!("{app_name}_nif"),
    );

    // `lib/` is populated when either (a) at least one non-OptionsField trait
    // bridge emits a GenServer module into `lib/`, or (b) a wrapper module file
    // (`lib/<app_name>.ex`, or any `.ex` under `lib/`) was emitted earlier in
    // the pipeline. Hex publish refuses to package a non-existent directory,
    // but it equally fails to publish a usable hex package when the wrapper
    // module exists on disk yet is excluded from `files:` — so include `lib`
    // whenever anything in `lib/` already exists.
    let lib_has_files_on_disk = {
        let lib_dir_rel = format!("{pkg_dir}/lib");
        let lib_dir = if let Some(ws_root) = config.workspace_root.as_deref() {
            ws_root.join(&lib_dir_rel)
        } else {
            PathBuf::from(&lib_dir_rel)
        };
        // Treat the lib dir as "populated" if it contains at least one .ex file
        // (recursively). The wrapper module is alef-emitted earlier in the
        // pipeline; by the time mix.exs is rendered it should already be on
        // disk. A nested `.ex` (e.g. submodule under `lib/<app>/foo.ex`) also
        // counts — those need packaging too.
        fn has_any_ex_file(dir: &std::path::Path) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if has_any_ex_file(&path) {
                        return true;
                    }
                } else if path.extension().is_some_and(|e| e == "ex") {
                    return true;
                }
            }
            false
        }
        has_any_ex_file(&lib_dir)
    };
    let lib_populated = lib_has_files_on_disk
        || config.trait_bridges.iter().any(|b| {
            !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler")
                && b.bind_via != BridgeBinding::OptionsField
        });

    // Always-present entries on disk after scaffolding: native crate sources,
    // .formatter.exs, mix.exs, and the README (alef writes one or expects one).
    // `lib` is conditional (see above). The native dir is narrowed to source
    // files only (Cargo.toml/Cargo.lock/src + optional build.rs) to keep
    // `target/` build artifacts out of the hex tarball — `mix hex.publish`
    // packs everything listed here, and a populated `target/` can blow past
    // hex's `metadata.config` size limit. `checksum-*.exs` is included so
    // RustlerPrecompiled can verify dynamically-downloaded NIFs on the
    // consumer side without forcing a source build. `build.rs` is conditional —
    // include it only when the NIF crate actually has one, otherwise
    // `mix hex.publish` fails with `Missing files: native/<nif>/build.rs`.
    let mut files_entries: Vec<String> = vec![
        ".formatter.exs".into(),
        "mix.exs".into(),
        "README*".into(),
        "checksum-*.exs".into(),
        format!("native/{nif_name}/Cargo.toml"),
        format!("native/{nif_name}/Cargo.lock"),
    ];

    // The NIF crate's `[lib] path` (see `scaffold_elixir_cargo`) determines
    // where the Rust source lives. Check if the standard native/<nif>/src exists
    // on disk:
    // - If it does: use it (standard monorepo layout, self-contained).
    // - If it doesn't: the Rust source is elsewhere (either at an external path
    //   configured via `[crate.output] elixir`, or co-located with the wrapper
    //   module in lib/). We must list the directory containing the actual lib.rs
    //   or the hex tarball will be incomplete and RustlerPrecompiled's build
    //   fallback will fail. Omit it here if `lib_populated` will add it below.
    if let Some(ws_root) = config.workspace_root.as_deref() {
        let native_src_dir_rel = format!("{pkg_dir}/native/{nif_name}/src");
        let native_src_dir = ws_root.join(&native_src_dir_rel);
        if native_src_dir.exists() {
            files_entries.push(format!("native/{nif_name}/src"));
        } else if let Some(relative) = external_elixir_src.as_deref() {
            // External source: list the directory containing the actual lib.rs.
            files_entries.push(relative.to_string());
        } else if !lib_populated {
            // The NIF source is co-located with the generated wrapper module in lib/.
            files_entries.push("lib".to_string());
        }
    } else if let Some(relative) = external_elixir_src.as_deref() {
        files_entries.push(relative.to_string());
    }
    // Note: if neither condition above is met (no workspace_root, no external source,
    // and native/<nif>/src doesn't exist), we omit the Rust source from files:.
    // This avoids listing nonexistent paths that would cause `mix hex.publish` to fail.
    // The NIF source will be included via other means (e.g., as part of "lib" if
    // the Elixir source is co-located with it).

    let native_crate_dir_rel = format!("{pkg_dir}/native/{nif_name}");
    let build_rs_path = if let Some(ws_root) = config.workspace_root.as_deref() {
        ws_root.join(&native_crate_dir_rel).join("build.rs")
    } else {
        PathBuf::from(&native_crate_dir_rel).join("build.rs")
    };
    if build_rs_path.exists() {
        files_entries.push(format!("native/{nif_name}/build.rs"));
    }
    if lib_populated {
        files_entries.insert(0, "lib".into());
    }
    let files_line = files_entries.join(" ");

    // When the files: line would exceed mix format's default 98-char limit,
    // emit a wrapped form that mix format is stable with.
    let files_keyword = if files_line.len() > 85 {
        // Wrap: emit ~w() on a new line with indentation
        let files_entries_str = files_entries.join(" ");
        format!("\n        ~w({})", files_entries_str)
    } else {
        format!("~w({})", files_line)
    };
    let links_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("links: %{{\"GitHub\" => \"{repository}\"}},"))
        .unwrap_or_default();
    let license = meta.license.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Elixir scaffold requires package metadata license; set package_metadata.license or scaffold.license"
        )
    })?;

    let content = format!(
        r#"defmodule {module}.MixProject do
  use Mix.Project

  def project do
    [
      app: :{app_name},
      version: "{version}",
      elixir: "~> 1.14",{elixirc_paths}
      {rustler_crates_block}
      description: "{description}",
      package: package(),
      deps: deps()
    ]
  end

  defp package do
    [
      licenses: ["{license}"],
      {links}
      files:{files_keyword}
    ]
  end

  defp deps do
    [{jason_dep}
      {{:rustler, "{rustler_hex}", runtime: false}},
      {{:rustler_precompiled, "{rustler_precompiled}"}},
      {{:credo, "{credo}", only: [:dev, :test], runtime: false}},
      {{:ex_doc, "{ex_doc}", only: :dev, runtime: false}}
    ]
  end
end
"#,
        module = app_name.to_pascal_case(),
        app_name = app_name,
        version = version,
        elixirc_paths = elixirc_paths_line,
        rustler_crates_block = rustler_crates_block,
        files_keyword = files_keyword,
        jason_dep = jason_dep,
        description = meta.description,
        license = license,
        links = links_line,
        rustler_hex = tv::hex::RUSTLER,
        rustler_precompiled = tv::hex::RUSTLER_PRECOMPILED,
        credo = tv::hex::CREDO,
        ex_doc = tv::hex::EX_DOC,
    );

    let formatter_content = r#"[
  import_deps: [:rustler],
  inputs: ["{mix,.formatter}.exs", "{config,lib,test}/**/*.{ex,exs}"],
  line_length: 140
]
"#;

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/mix.exs")),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.formatter.exs")),
            content: formatter_content.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.credo.exs")),
            content: r#"%{
  configs: [
    %{
      name: "default",
      strict: true,
      parse_timeout: 5000,
      files: %{
        included: [
          "lib/",
          "src/",
          "test/",
          "web/",
          "apps/*/lib/",
          "apps/*/src/",
          "apps/*/test/",
          "apps/*/web/"
        ],
        excluded: [
          ~r"/_build/",
          ~r"/deps/",
          ~r"/node_modules/"
        ]
      },
      checks: %{
        enabled: [
          {Credo.Check.Refactor.CyclomaticComplexity, max_complexity: 16}
        ]
      }
    }
  ]
}
"#
            .to_string(),
            generated_header: false,
        },
    ];

    // Generate trait bridge GenServer modules.
    // Options-field bridges use inline visitor maps passed alongside the call — they do
    // not use a registered GenServer, so skip them here.
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .iter()
            .any(|l| l == "elixir" || l == "rustler")
        {
            continue;
        }
        // Skip options-field bridges — visitor is passed inline, no GenServer needed.
        if bridge_cfg.bind_via == BridgeBinding::OptionsField {
            continue;
        }
        let trait_name_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_name_camel = capitalize_first(&bridge_cfg.trait_name);
        let module_name = format!("{}{}Bridge", app_name.to_pascal_case(), trait_name_camel);
        let native_mod = format!("{}.Native", app_name.to_pascal_case());

        let bridge_content = format!(
            r#"defmodule {module_name} do
  @moduledoc """
  GenServer bridge for {trait_name} implementation in {app_name}.

  Handles incoming trait method calls from Rust and dispatches them to an implementation module.
  """

  use GenServer

  require Logger

  @doc """
  Start a GenServer linked to the current process.

  impl_module should be a module that implements the {trait_name} trait methods.
  """
  def start_link(impl_module) do
    GenServer.start_link(__MODULE__, impl_module, name: __MODULE__)
  end

  @impl GenServer
  def init(impl_module) do
    {{:ok, impl_module}}
  end

  @doc """
  Handle an incoming trait call message.

  Message format: {{:trait_call, method_atom, args_json, reply_id}}
  """
  @impl GenServer
  def handle_info({{:trait_call, method, args_json, reply_id}}, impl_module) do
    try do
      args = Jason.decode!(args_json)
      method_name = to_string(method)
      ordered_args = ordered_args(impl_module, method_name, args)

      # Dispatch to the implementation module
      result = apply(impl_module, String.to_existing_atom(method_name), ordered_args)

      # Send result back to Rust
      {native_mod}.complete_trait_call(reply_id, Jason.encode!(result))
    rescue
      e ->
        Logger.error("Error calling {{impl_module}}.{{method}}: {{Exception.message(e)}}")
        {native_mod}.fail_trait_call(reply_id, Exception.message(e))
    end

    {{:noreply, impl_module}}
  end

  defp ordered_args(impl_module, method_name, args) when is_map(args) do
    if function_exported?(impl_module, :__alef_arg_order__, 1) do
      impl_module.__alef_arg_order__(method_name)
      |> Enum.map(&Map.fetch!(args, &1))
    else
      args
      |> Map.keys()
      |> Enum.sort()
      |> Enum.map(&Map.fetch!(args, &1))
    end
  end

  defp ordered_args(_impl_module, _method_name, args) when is_list(args), do: args

  @doc """
  Register an implementation module, starting a GenServer to handle trait calls.
  """
  def register(impl_module) do
    plugin_name = impl_module.name()
    {{:ok, pid}} = start_link(impl_module)
    {native_mod}.register_{trait_name_snake}(pid, plugin_name)
  end
end
"#,
            module_name = module_name,
            trait_name = bridge_cfg.trait_name,
            app_name = app_name,
            trait_name_snake = trait_name_snake,
            native_mod = native_mod,
        );

        let bridge_path = PathBuf::from(format!("{pkg_dir}/lib/{app_name}/{trait_name_snake}_bridge.ex"));
        files.push(GeneratedFile {
            path: bridge_path,
            content: bridge_content,
            generated_header: true,
        });
    }

    Ok(files)
}

fn elixir_nif_targets(config: &ResolvedCrateConfig) -> Vec<String> {
    config
        .elixir
        .as_ref()
        .filter(|elixir| !elixir.nif_targets.is_empty())
        .map(|elixir| elixir.nif_targets.clone())
        .unwrap_or_else(|| {
            [
                "aarch64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
                "x86_64-pc-windows-gnu",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        })
}
