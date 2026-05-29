use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use std::path::PathBuf;

pub(crate) fn scaffold_ruby_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Ruby);
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-rb"), version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Ruby);

    // Check if trait bridges are configured and add async-trait if needed
    let has_trait_bridges = !config.trait_bridges.is_empty();
    // Streaming adapters require `futures` (BoxStream + StreamExt) in the
    // generated Magnus iterator wrapper.
    let has_streaming_adapter = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming));
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    // ahash is needed when any function takes an AHashMap<Cow, _> param — the generated
    // Magnus wrapper emits a `let __<name>_ahash: ahash::AHashMap<...>` pre-call binding.
    let needs_ahash = api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let lib_name = format!("{}_rb", core_crate_dir.replace('-', "_"));

    // Collect all [dependencies] entries then sort alphabetically so the emitted
    // Cargo.toml is cargo-sort canonical without a post-processing step.
    let features_str = core_dep_features(config, Language::Ruby);
    let mut dep_lines: Vec<String> = vec![
        crate::scaffold::render_core_dep(
            &config.name,
            &format!("../../../../../crates/{core_crate_dir}"),
            &features_str,
            version,
        ),
        format!("magnus = \"{}\"", tv::cargo::MAGNUS),
        "serde = { version = \"1\", features = [\"derive\"] }".to_owned(),
        "serde_json = \"1\"".to_owned(),
    ];
    if has_async || has_trait_bridges {
        dep_lines.push("tokio = { version = \"1\", features = [\"rt-multi-thread\"] }".to_owned());
    }
    if needs_ahash && !dep_lines.iter().any(|l| l.starts_with("ahash")) {
        dep_lines.push("ahash = \"0.8\"".to_owned());
    }
    if has_trait_bridges && !dep_lines.iter().any(|l| l.starts_with("async-trait")) {
        dep_lines.push("async-trait = \"0.1\"".to_owned());
    }
    if has_streaming_adapter && !dep_lines.iter().any(|l| l.starts_with("futures")) {
        dep_lines.push("futures = \"0.3\"".to_owned());
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
    dep_lines.sort();
    let deps_section = dep_lines.join("\n");

    // Build the cargo-machete ignored list. When `tokio` is included in
    // dependencies (for async, trait bridges, or streaming), mark it as ignored
    // since the Magnus wrapper may not directly reference it. Same for
    // `async-trait` (included for trait bridges) and `futures` (included for
    // streaming adapters). `ahash` is added when any function parameter uses
    // AHashMap<Cow, _>, but the Magnus wrapper never directly uses ahash—it's
    // used only in the Rust core for type field marshalling.
    let mut machete_ignored: Vec<&str> = Vec::new();
    if has_async || has_trait_bridges {
        machete_ignored.push("tokio");
    }
    if has_trait_bridges {
        machete_ignored.push("async-trait");
    }
    if has_streaming_adapter {
        machete_ignored.push("futures");
    }
    if needs_ahash {
        machete_ignored.push("ahash");
    }
    // cargo-sort places `[package.metadata.*]` immediately after `[package]`,
    // before `[lib]` / `[dependencies]`.
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

    let content = format!(
        r#"{pkg_header}

{machete_section}[lib]
name = "{lib_name}"
path = "../src/lib.rs"
crate-type = ["cdylib"]

[dependencies]
{deps_section}"#,
        pkg_header = pkg_header,
        machete_section = machete_section,
        lib_name = lib_name,
        deps_section = deps_section,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!(
            "{pkg_dir}/ext/{}_rb/native/Cargo.toml",
            core_crate_dir.replace('-', "_")
        )),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_ruby(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let gem_name = config.ruby_gem_name();
    // File paths must use snake_case (Ruby convention), even if gem_name has hyphens.
    let gem_name_snake = gem_name.replace('-', "_");
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Ruby);
    // The native extension name uses the core crate dir with underscores and _rb suffix,
    // matching the directory generated by scaffold_ruby_cargo: ext/{core_crate_dir}_rb/
    let ext_name = format!("{}_rb", core_crate_dir.replace('-', "_"));
    // The cargo package name for RbSys::ExtensionTask lookup uses hyphens per binding-crate convention.
    let cargo_pkg_name = format!("{}-rb", core_crate_dir);
    // Gemspec versions must use RubyGems prerelease form (`1.8.0.pre.rc.2`),
    // not cargo's dash-form (`1.8.0-rc.2`) which `gem build` rejects.
    let version = crate::core::version::to_rubygems_prerelease(&api.version);

    let authors_ruby = if meta.authors.is_empty() {
        "[]".to_string()
    } else {
        let entries: Vec<String> = meta.authors.iter().map(|a| format!("\"{}\"", a)).collect();
        format!("[{}]", entries.join(", "))
    };

    let metadata_ruby = if meta.keywords.is_empty() {
        String::new()
    } else {
        // Rubocop's `Style/WordArray` autocorrects bare-identifier string arrays to `%w[...]`.
        // Emit the canonical form directly when every keyword is a single word with no
        // shell-special chars; otherwise fall back to a double-quoted array literal to
        // match `Style/StringLiterals: double_quotes`.
        let word_array_safe = meta
            .keywords
            .iter()
            .all(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let array_literal = if word_array_safe {
            format!("%w[{}]", meta.keywords.join(" "))
        } else {
            let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
            format!("[{}]", entries.join(", "))
        };
        format!("  spec.metadata[\"keywords\"] = {}.join(\",\")\n", array_literal)
    };

    let content = format!(
        r#"# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name = "{gem_name}"
  spec.version = "{version}"
  spec.authors       = {authors}
  spec.summary       = "{description}"
  spec.description   = "{description}"
  spec.homepage      = "{repository}"
  spec.license       = "{license}"
  spec.required_ruby_version = ">= 3.2.0"
{metadata}  spec.metadata["rubygems_mfa_required"] = "true"

  spec.files         = Dir.glob(%w[README* LICENSE* lib/**/* ext/**/* sig/**/* Steepfile]).reject {{ |f| f.include?("/native/target/") || f.include?("/native/tmp/") }}
  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/{ext_name}/extconf.rb"]

  spec.add_dependency "rb_sys", "{rb_sys}"
  spec.add_dependency "sorbet-runtime", "{sorbet_runtime}"
end
"#,
        gem_name = gem_name,
        ext_name = ext_name,
        version = version,
        authors = authors_ruby,
        description = meta.description,
        repository = meta.repository,
        license = meta.license,
        metadata = metadata_ruby,
        rb_sys = tv::gem::RB_SYS,
        sorbet_runtime = tv::gem::SORBET_RUNTIME,
    );

    let rubocop_content = r#"plugins:
  - rubocop-performance
  - rubocop-rspec

AllCops:
  TargetRubyVersion: 3.2
  NewCops: enable
  SuggestExtensions: false
  Exclude:
    - "vendor/**/*"
    - "tmp/**/*"
    - "lib/**/*.bundle"
    - "lib/**/*.rb"
    - "ext/**/*"

Style/FrozenStringLiteralComment:
  Enabled: true
  EnforcedStyle: always

Style/StringLiterals:
  Enabled: true
  EnforcedStyle: double_quotes

Style/StringLiteralsInInterpolation:
  Enabled: true
  EnforcedStyle: double_quotes

Style/Documentation:
  Enabled: false

Layout/LineLength:
  Max: 120
  AllowedPatterns:
    - '\A\s*#'
  Exclude:
    - "spec/**/*"

Metrics/MethodLength:
  Max: 20
  Exclude:
    - "spec/**/*"

Metrics/BlockLength:
  Enabled: true
  Max: 350
  CountComments: false

Metrics/AbcSize:
  Max: 20
  Exclude:
    - "spec/**/*"

RSpec/ExampleLength:
  Max: 50

RSpec/MultipleExpectations:
  Max: 25

RSpec/NestedGroups:
  Max: 6
"#
    .to_string();

    let rakefile_content = format!(
        r#"# frozen_string_literal: true

require "bundler"
Bundler::GemHelper.install_tasks name: "{gem_name_snake}"
require "rb_sys/extensiontask"
require "rspec/core/rake_task"

GEM_ROOT = __dir__
GEMSPEC = Gem::Specification.load(File.expand_path("{gem_name_snake}.gemspec", GEM_ROOT))

# RbSys::ExtensionTask runs `cargo metadata` from Dir.pwd to look up the
# native crate. The native binding crate at ext/{ext_name}/native is excluded
# from the parent Cargo workspace (so the parent does not pull in rb_sys
# deps), so the lookup raises PackageNotFoundError when the Rakefile runs
# from the gem root. Run init from the native dir so cargo metadata finds the
# standalone Cargo.toml; absolute lib_dir keeps the rake task pointing at the
# gem root.
EXT_NATIVE_DIR = File.expand_path("ext/{ext_name}/native", GEM_ROOT)

Dir.chdir(EXT_NATIVE_DIR) do
  RbSys::ExtensionTask.new("{cargo_pkg_name}", GEMSPEC) do |ext|
    ext.lib_dir = File.expand_path("lib", GEM_ROOT)
    ext.ext_dir = EXT_NATIVE_DIR
    ext.cross_compile = true
    ext.cross_platform = %w[
      x86_64-linux
      aarch64-linux
      x86_64-darwin
      arm64-darwin
      x64-mingw32
      x64-mingw-ucrt
    ]
  end
end

RSpec::Core::RakeTask.new(:spec)

task spec: :compile
task default: :spec
"#,
        gem_name_snake = gem_name_snake,
        cargo_pkg_name = cargo_pkg_name,
        ext_name = ext_name,
    );

    let extconf_content = format!(
        r#"# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

default_profile = ENV.fetch("CARGO_PROFILE", "release")

create_rust_makefile("{ext_name}") do |config|
  config.profile = default_profile.to_sym
  config.ext_dir = "native"
end
"#,
        ext_name = ext_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/{}.gemspec", gem_name_snake)),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.rubocop.yml")),
            content: rubocop_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/Rakefile")),
            content: rakefile_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/ext/{ext_name}/extconf.rb", ext_name = ext_name)),
            content: extconf_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/Gemfile")),
            content: format!(
                r#"# frozen_string_literal: true

source "https://rubygems.org"

gemspec

group :development do
  gem "rake-compiler", "{rake_compiler}"
  gem "rb_sys", "{rb_sys}"
  gem "rspec", "{rspec}"
  gem "rubocop", "{rubocop}"
  gem "rubocop-performance", "{rubocop_performance}"
  gem "rubocop-rspec", "{rubocop_rspec}"
  gem "steep", "{steep}"
end
"#,
                rake_compiler = tv::gem::RAKE_COMPILER,
                rb_sys = tv::gem::RB_SYS,
                rspec = tv::gem::RSPEC_SCAFFOLD,
                rubocop = tv::gem::RUBOCOP_SCAFFOLD,
                rubocop_performance = tv::gem::RUBOCOP_PERFORMANCE,
                rubocop_rspec = tv::gem::RUBOCOP_RSPEC_SCAFFOLD,
                steep = tv::gem::STEEP,
            ),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/Steepfile")),
            content: format!(
                r#"# frozen_string_literal: true

target :lib do
  signature "sig"
  check "lib"
  # The generated `lib/{gem_name_snake}/native.rb` carries inline Sorbet
  # `sig {{ ... }}` blocks on tagged-enum variant Data classes. Sorbet's runtime
  # provides those via `extend T::Sig`, but Steep does not understand the
  # extension (it relies on RBS, not Sorbet sigs) and reports
  # `Type `self` does not have method `sig`` on every block. RBS coverage
  # for the same surface lives in `sig/types.rbs`, so we steer Steep to the
  # RBS file by ignoring the .rb.
  ignore "lib/{gem_name_snake}/native.rb"
end
"#,
                gem_name_snake = gem_name_snake,
            ),
            generated_header: false,
        },
    ])
}
