use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, BridgeBinding, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::capitalize_first, scaffold::cargo_package_header, scaffold::detect_workspace_inheritance_for_crate,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

/// The Elixir native crate's directory (relative to the project root), e.g.
/// `packages/elixir/native/my_lib_nif`.
///
/// Single source of truth for that formula: `scaffold_elixir_cargo` writes the crate's
/// Cargo.toml here, and a caller wanting to read that manifest back (e.g.
/// `RustlerBackend::generate_bindings`, cross-checking it against
/// `codegen::cfg::collect_cfg_features`) must derive the exact same path rather than
/// re-deriving it from an unrelated path such as `alef build`'s own `lib.rs` output directory --
/// which a `[crates.output] elixir = "..."` override can point somewhere this formula's `nif_name`
/// segment does not appear under at all. ~keep
pub(crate) fn elixir_native_crate_dir(config: &ResolvedCrateConfig) -> String {
    let nif_name = format!("{}_nif", config.elixir_app_name());
    let pkg_dir = config.package_dir(Language::Elixir);
    format!("{pkg_dir}/native/{nif_name}")
}

pub(crate) fn scaffold_elixir_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let nif_name = format!("{app_name}_nif");
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let native_crate_dir = elixir_native_crate_dir(config);
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &native_crate_dir);
    let pkg_header = cargo_package_header(&nif_name, version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Elixir);
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    let has_trait_bridges = config
        .trait_bridges
        .iter()
        .any(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"));
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
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

    let excluded_default_features: std::collections::HashSet<&str> = config
        .elixir
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let features_str =
        crate::scaffold::core_dep_features_excluding(config, Language::Elixir, &excluded_default_features);
    let core_overrides = config
        .elixir
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let (core_dep_line, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &format!("../../../../crates/{core_crate_dir}"),
        &features_str,
        version,
        core_overrides,
    );
    let core_target_blocks_section = if core_target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{core_target_blocks}")
    };
    let mut dep_lines: Vec<String> = vec![
        format!("rustler = \"{}\"", tv::cargo::RUSTLER),
        "serde = { version = \"1\", features = [\"derive\"] }".to_owned(),
        "serde_json = \"1\"".to_owned(),
    ];
    if needs_ahash {
        dep_lines.push("ahash = \"0.8\"".to_owned());
    }
    if has_trait_bridges {
        dep_lines.push(format!("async-trait = \"{}\"", tv::cargo::ASYNC_TRAIT));
        dep_lines.push(format!("tracing = \"{}\"", tv::cargo::TRACING));
    }
    if has_async || has_trait_bridges || has_streaming {
        dep_lines.push("tokio = { version = \"1\", features = [\"rt-multi-thread\", \"sync\"] }".to_owned());
    }
    if has_streaming && !dep_lines.iter().any(|l| l.starts_with("futures-util")) {
        dep_lines.push("futures-util = \"0.3\"".to_owned());
    }
    if !config.components.is_empty() {
        let alef_version = env!("CARGO_PKG_VERSION");
        for (name, dependency) in [
            ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
            (
                "alef-component-runtime",
                format!("alef-component-runtime = \"{alef_version}\""),
            ),
            ("directories", "directories = \"6\"".to_owned()),
        ] {
            let configured = dep_lines.iter().map(String::as_str).chain(extra_deps.lines());
            if !crate::scaffold::cargo_dependency_declared(configured, name) {
                dep_lines.push(dependency);
            }
        }
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
    dep_lines.push("alloc-no-stdlib = \"=2.0.4\"".to_owned());
    dep_lines.push("alloc-stdlib = \"=0.2.2\"".to_owned());
    dep_lines.push("brotli-decompressor = \"=5.0.1\"".to_owned());
    if !core_dep_line.is_empty() {
        dep_lines.push(core_dep_line);
    }
    crate::scaffold::sort_dependency_lines(&mut dep_lines);
    let deps_section = dep_lines.join("\n");

    let mut machete_ignored: Vec<&str> = Vec::new();
    if has_async || has_trait_bridges || has_streaming {
        machete_ignored.push("tokio");
    }
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tracing");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    if needs_ahash {
        machete_ignored.push("ahash");
    }
    machete_ignored.push("alloc-no-stdlib");
    machete_ignored.push("alloc-stdlib");
    machete_ignored.push("brotli-decompressor");
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
    let referenced_features = crate::codegen::cfg::collect_cfg_features(api);

    // No `[crates.elixir] nif_features` override -> mirror the core crate's own declared
    // `[features] default = [...]` list rather than any alef-side guess at feature identity.
    // A consumer whose core crate declares no defaults simply forwards none. ~keep
    let base_features: std::collections::BTreeSet<String> =
        match config.elixir.as_ref().and_then(|c| c.nif_features.clone()) {
            Some(nif_features) => nif_features.into_iter().collect(),
            None => crate::scaffold::core_feature_closure(config, &[]).1,
        };
    let mut always_features: std::collections::BTreeSet<String> = base_features;
    always_features.extend(referenced_features.clone());
    // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`, not
    // listed in `nif_features`, and not a canonical default present in the core crate's
    // manifest) must still get a forwarding entry below -- alef-task #374, regression in
    // `cargo_excluded_features_tests`. ~keep
    always_features.extend(excluded_default_features.iter().map(|name| (*name).to_string()));

    // features to the core crate. Without this, #[cfg(feature = "X")] arms fail
    //
    // A name in `excluded_default_features` is still declared below (so `cargo build --features
    // <name>` keeps working) but dropped from `default`, matching
    // `RubyConfig::excluded_default_features`. ~keep
    let features_table = {
        let lines = crate::codegen::cfg::cfg_default_and_forwarding_lines(
            &always_features,
            &config.name,
            &excluded_default_features,
        );
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    let check_cfg_block = {
        let csv = always_features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        // The `unexpected_cfgs` line stays a hand-written literal rather than routing
        // through `CargoLintsConfig::render` -- Cargo allows only one `[lints.rust]`
        // table per manifest, so a configured `[crates.cargo_lints.rust]` entry has
        // to become an extra sibling line under this same header, not a second one. ~keep
        let mut rust_lines = vec![format!(
            "unexpected_cfgs = {{ level = \"warn\", check-cfg = ['cfg(feature, values({csv}))'] }}"
        )];
        rust_lines.extend(config.cargo_lints.extra_rust_lines(&["unexpected_cfgs"]));
        let clippy_block = crate::scaffold::cargo_lints_clippy_block_with_rationale(config);
        let clippy_section = if clippy_block.is_empty() {
            String::new()
        } else {
            format!("\n\n{clippy_block}")
        };
        format!("\n[lints.rust]\n{}{clippy_section}\n", rust_lines.join("\n"))
    };

    let content = format!(
        r#"{pkg_header}

{machete_section}[workspace]

[lib]
name = "{nif_name}"
{lib_path_line}
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_section}{core_target_blocks_section}{check_cfg_block}"#,
        pkg_header = pkg_header,
        machete_section = machete_section,
        nif_name = nif_name,
        lib_path_line = lib_path_line,
        features_table = features_table,
        check_cfg_block = check_cfg_block,
        deps_section = deps_section,
        core_target_blocks_section = core_target_blocks_section,
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

    let jason_dep = format!("\n      {{:jason, \"{jason}\"}},", jason = tv::hex::JASON);

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

    let lib_has_files_on_disk = {
        let lib_dir_rel = format!("{pkg_dir}/lib");
        let lib_dir = if let Some(ws_root) = config.workspace_root.as_deref() {
            ws_root.join(&lib_dir_rel)
        } else {
            PathBuf::from(&lib_dir_rel)
        };
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

    let mut files_entries: Vec<String> = vec![
        ".formatter.exs".into(),
        "mix.exs".into(),
        "README*".into(),
        "checksum-*.exs".into(),
        format!("native/{nif_name}/Cargo.toml"),
        format!("native/{nif_name}/Cargo.lock"),
    ];

    if let Some(ws_root) = config.workspace_root.as_deref() {
        let native_src_dir_rel = format!("{pkg_dir}/native/{nif_name}/src");
        let native_src_dir = ws_root.join(&native_src_dir_rel);
        if native_src_dir.exists() {
            files_entries.push(format!("native/{nif_name}/src"));
        } else if let Some(relative) = external_elixir_src.as_deref() {
            files_entries.push(relative.to_string());
        } else if !lib_populated {
            files_entries.push("lib".to_string());
        }
    } else if let Some(relative) = external_elixir_src.as_deref() {
        files_entries.push(relative.to_string());
    }

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

    const FILES_LINE_WRAP_OVERHEAD: usize = 17;
    const FORMATTER_LINE_LENGTH: usize = 140;
    let files_keyword = if files_line.len() + FILES_LINE_WRAP_OVERHEAD > FORMATTER_LINE_LENGTH {
        format!("\n        ~w({files_line})")
    } else {
        format!(" ~w({files_line})")
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

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .iter()
            .any(|l| l == "elixir" || l == "rustler")
        {
            continue;
        }
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

  Message format: {{:trait_call, method_atom, args, reply_id}}

  `args` arrives as a native Erlang map (no JSON decode); the reply stays JSON.
  """
  @impl GenServer
  def handle_info({{:trait_call, method, args, reply_id}}, impl_module) do
    try do
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

    # Names of the functions the implementation module exports. Rust-defaulted
    # trait methods outside this list keep their Rust default behavior instead
    # of being dispatched to the module.
    implemented_methods =
      impl_module.__info__(:functions)
      |> Enum.map(fn {{name, _arity}} -> Atom.to_string(name) end)
      |> Enum.uniq()

    {native_mod}.register_{trait_name_snake}(pid, plugin_name, implemented_methods)
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

#[cfg(test)]
mod cargo_excluded_features_tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    /// Regression for alef-task #374: an `excluded_default_features` name that gates no item in
    /// the extracted API surface (e.g. a Cargo-only feature that only affects a dependency's
    /// `build.rs` linking, such as `libheif-sys` via `heic`) is never discovered by
    /// `crate::codegen::cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]`
    /// attributes on IR nodes. `always_features` unions that discovery set with `base_features`
    /// (canonical defaults intersected with the core crate's real `Cargo.toml`, or an explicit
    /// `nif_features` override), but a name that is neither cfg-discoverable, nor listed in
    /// `nif_features`, nor a canonical default present in the core crate's manifest never reaches
    /// `always_features` at all -- breaking `mix compile --force` with `--features <name>`-style
    /// opt-in, exactly the escape hatch `excluded_default_features` documents as always
    /// available. Deliberately picks `heic` (not one of the hardcoded canonical defaults
    /// `download`/`serde`/`config`) and configures no `nif_features`, so `base_features` cannot
    /// smuggle it in either. `get_core_crate_features` also finds no real `Cargo.toml` on disk for
    /// this fixture's `workspace_root`, so it contributes nothing here.
    #[test]
    fn scaffold_elixir_cargo_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "sample-lib"
sources = []
[crates.elixir]
excluded_default_features = ["heic"]
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };

        let files = scaffold_elixir_cargo(&api, &config).expect("scaffold_elixir_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        assert!(
            cargo_toml.contains(r#"heic = ["sample-lib/heic"]"#),
            "a config-only excluded_default_features name (not referenced by any \
             #[cfg(feature = ...)] in the API surface, not in `nif_features`, and not a canonical \
             default present in the core crate's manifest) must still get a forwarding entry so \
             opting back in keeps working:\n{cargo_toml}"
        );
        let default_line = cargo_toml
            .lines()
            .find(|line| line.starts_with("default = ["))
            .expect("default array present");
        assert!(
            !default_line.contains("heic"),
            "default = [...] must NOT contain excluded `heic`; got: {default_line}"
        );
    }
}

#[cfg(test)]
mod path_safety_tests {
    use super::*;
    use crate::core::config::{NewAlefConfig, TraitBridgeConfig};

    #[test]
    fn elixir_trait_bridge_sink_fires_with_a_contained_app_path() {
        let parsed: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "sample-core"
sources = []
[crates.elixir]
app_name = "safe_app"
[crates.scaffold]
license = "MIT"
"#,
        )
        .expect("valid config");
        let mut config = parsed.resolve().expect("resolve").remove(0);
        config.trait_bridges = vec![TraitBridgeConfig {
            trait_name: "Backend".into(),
            ..TraitBridgeConfig::default()
        }];

        let files = scaffold_elixir(&ApiSurface::default(), &config).expect("Elixir scaffold renders");
        let bridge = files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with("safe_app/backend_bridge.ex"))
            .expect("the conditional trait-bridge sink must fire");
        crate::core::config::output::validate_output_path(&bridge.path).expect("trait-bridge path remains contained");
    }
}
