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

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming_adapter = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming));
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    let needs_ahash = api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let lib_name = format!("{}_rb", core_crate_dir.replace('-', "_"));

    let features_str = core_dep_features(config, Language::Ruby);
    let core_overrides = config
        .ruby
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let (core_dep_line, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &format!("../../../../../crates/{core_crate_dir}"),
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
        format!("magnus = \"{}\"", tv::cargo::MAGNUS),
        "rb-sys = \">=0.9, <0.9.128\"".to_owned(),
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
    if !core_dep_line.is_empty() {
        dep_lines.push(core_dep_line);
    }
    dep_lines.sort();
    let deps_section = dep_lines.join("\n");

    let mut machete_ignored: Vec<&str> = vec!["rb-sys"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        if !has_async {
            machete_ignored.push("tokio");
        }
    }
    machete_ignored.sort_unstable();
    let ignored_list = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let machete_section = format!("[package.metadata.cargo-machete]\nignored = [{ignored_list}]\n\n");

    // core dep. Without this, `#[cfg(feature = "X")]` arms emitted by the
    let cfg_features = crate::codegen::cfg::collect_cfg_features(api);
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        let mut lines: Vec<String> = Vec::with_capacity(cfg_features.len() + 1);
        let default_list: Vec<String> = cfg_features.iter().map(|name| format!("\"{name}\"")).collect();
        lines.push(format!("default = [{}]", default_list.join(", ")));
        for name in &cfg_features {
            lines.push(format!(
                r#"{name} = ["{core_dep_key}/{name}"]"#,
                core_dep_key = config.name
            ));
        }
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    let content = format!(
        r#"{pkg_header}

{machete_section}[lib]
name = "{lib_name}"
path = "../src/lib.rs"
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_section}{core_target_blocks_section}"#,
        pkg_header = pkg_header,
        machete_section = machete_section,
        lib_name = lib_name,
        features_table = features_table,
        deps_section = deps_section,
        core_target_blocks_section = core_target_blocks_section,
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
    let gem_name_snake = gem_name.replace('-', "_");
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Ruby);
    let ext_name = format!("{}_rb", core_crate_dir.replace('-', "_"));
    let cargo_pkg_name = format!("{}-rb", core_crate_dir);
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
    let homepage_ruby = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("  spec.homepage      = \"{repository}\"\n"))
        .unwrap_or_default();
    let license_ruby = meta
        .license
        .as_deref()
        .map(|license| format!("  spec.license       = \"{license}\"\n"))
        .unwrap_or_default();

    let content = format!(
        r#"# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name = "{gem_name}"
  spec.version = "{version}"
  spec.authors       = {authors}
  spec.summary       = "{description}"
  spec.description   = "{description}"
{homepage}
{license}
  spec.required_ruby_version = [">= 3.2.0", "< 4.0"]
{metadata}  spec.metadata["rubygems_mfa_required"] = "true"

  candidate_files    = Dir.glob(%w[README* LICENSE* lib/**/* ext/**/* sig/**/* Steepfile]).select {{ |f| File.file?(f) }}
  spec.files         = candidate_files.reject {{ |f| f.include?("/native/target/") || f.include?("/native/tmp/") }}
  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/{ext_name}/native/extconf.rb"]

  spec.add_dependency "rb_sys", {rb_sys}
  spec.add_dependency "sorbet-runtime", "{sorbet_runtime}"
end
"#,
        gem_name = gem_name,
        ext_name = ext_name,
        version = version,
        authors = authors_ruby,
        description = meta.description,
        homepage = homepage_ruby,
        license = license_ruby,
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

# Formatting — layout, indentation, line breaks, line length — is owned by
# rubyfmt (via poly). Disable rubocop's Layout department so the two do not
# fight; rubocop runs for correctness/style lint only (CI, toolchain-gated).
Layout:
  Enabled: false

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

    // Ruby cross-compile platforms, each paired with the Rust target triple that
    // backs it. A platform whose triple is disabled via the workspace `[targets]`
    // opt-out table is dropped from the generated `CROSS_PLATFORMS` list.
    const RUBY_CROSS_PLATFORMS: &[(&str, &str)] = &[
        ("x86_64-linux", "x86_64-unknown-linux-gnu"),
        ("aarch64-linux", "aarch64-unknown-linux-gnu"),
        ("arm64-darwin", "aarch64-apple-darwin"),
        ("x86_64-darwin", "x86_64-apple-darwin"),
        ("x64-mingw-ucrt", "x86_64-pc-windows-msvc"),
    ];
    let cross_platforms = RUBY_CROSS_PLATFORMS
        .iter()
        .filter(|(_, triple)| config.target_enabled(triple))
        .map(|(platform, _)| format!("  {platform}"))
        .collect::<Vec<_>>()
        .join("\n");

    let rakefile_content = format!(
        r#"# frozen_string_literal: true

require "bundler"
Bundler::GemHelper.install_tasks name: "{gem_name_snake}"
require "rb_sys/extensiontask"
require "rspec/core/rake_task"

# Absolute path to the gem package directory, used as the anchor for resolving
# the gemspec and the native extension's Cargo manifest.
GEM_ROOT = __dir__
# Loaded gemspec used by Rake::ExtensionTask to compile the native extension.
GEMSPEC = Gem::Specification.load(File.expand_path("{gem_name_snake}.gemspec", GEM_ROOT))

# Set of supported platform identifiers for native gem cross-compilation.
# Used by `rb_sys/extensiontask` to drive the `rake compile:<platform>` tasks
# that produce platform-specific prebuilt gems published alongside the source
# gem on RubyGems.
CROSS_PLATFORMS = %w[
{cross_platforms}
].freeze

# rb_sys 0.9.x's Cargo::Metadata runs `cargo metadata` without `--manifest-path`,
# so it resolves to whatever workspace contains cwd. In this monorepo the root
# workspace excludes our crate, so the lookup fails with PackageNotFoundError.
# Chdir-around-construction also doesn't work because Rake::ExtensionTask resolves
# its own paths (lib_dir, ext_dir, task wiring) at construction time relative to
# cwd, breaking the compile pipeline. Patch Cargo::Metadata#cargo_metadata to add
# the explicit `--manifest-path` pointing at the crate's Cargo.toml so the lookup
# is unambiguous regardless of cwd.
MANIFEST_PATH = File.expand_path("ext/{ext_name}/native/Cargo.toml", GEM_ROOT)

# @!visibility private
module RbSys
  # @!visibility private
  module Cargo
    # @!visibility private
    class Metadata
      manifest_path = MANIFEST_PATH
      define_method(:cargo_metadata) do
        return @cargo_metadata if @cargo_metadata

        cargo = ENV["CARGO"] || "cargo"
        args = ["metadata", "--format-version", "1", "--manifest-path", manifest_path]
        args << "--no-deps" unless @deps
        out, stderr, status = Open3.capture3(cargo, *args)
        out.force_encoding(Encoding::UTF_8)
        raise "exited with non-zero status (#{{status}})" unless status.success?

        data = JSON.parse(out)
        raise "metadata must be a Hash" unless data.is_a?(Hash)

        @cargo_metadata = data
      rescue StandardError => e
        raise CargoMetadataError.new(e, stderr)
      end
      private :cargo_metadata
    end
  end
end

RbSys::ExtensionTask.new("{cargo_pkg_name}", GEMSPEC) do |ext|
  ext.lib_dir = "lib"
  ext.ext_dir = "ext/{ext_name}/native"
  ext.source_pattern = "*.{{}}"
  ext.platform = "ruby"
  ext.cross_compile = true
  ext.cross_platform = CROSS_PLATFORMS
  # Pin cross_compile_versions to Ruby 3.2-3.5 stable releases.
  # This overrides the container's RUBY_CC_VERSION env var at rake task definition time.
  # The setter was added in a later rb_sys version; guard against older gem installations
  # where the method does not exist (e.g., rb_sys 0.9.127 locked to avoid mingw bug in 0.9.128).
  # rb-sys-dock 0.9.x ships images for Ruby 3.2, 3.3, 3.4, and 3.5; this list must
  # match those available images. Per-ABI platform gem windows are controlled by rake-compiler-dock.
  ext.cross_compile_versions = %w[3.5.0 3.4.9 3.3.11 3.2.11] if ext.respond_to?(:cross_compile_versions=)
end

RSpec::Core::RakeTask.new(:spec)

# rake-compiler's `compile` task is a no-op when cross_compile is true; the real
# work hangs off `compile:<ruby_platform>`. Wire `compile` → `compile:ruby` so
# both the dev shorthand and CI's `bundle exec rake compile` actually build.
task compile: "compile:ruby"

task spec: :compile
task default: :spec
"#,
        gem_name_snake = gem_name_snake,
        cargo_pkg_name = cargo_pkg_name,
        ext_name = ext_name,
        cross_platforms = cross_platforms,
    );

    let extconf_content = format!(
        r#"# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

default_profile = ENV.fetch("CARGO_PROFILE", "release")

create_rust_makefile("{ext_name}") do |config|
  config.profile = default_profile.to_sym
  # extconf.rb and Cargo.toml are siblings under ext/{ext_name}/native/; rb_sys interprets
  # ext_dir relative to extconf.rb, so "." finds the sibling Cargo.toml. "native" would
  # resolve to native/native/Cargo.toml and break `gem install` on end-user machines.
  config.ext_dir = "."
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
            path: PathBuf::from(format!(
                "{pkg_dir}/ext/{ext_name}/native/extconf.rb",
                ext_name = ext_name
            )),
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
  gem "rb_sys", {rb_sys}
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
