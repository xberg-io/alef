use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::detect_workspace_inheritance_for_crate, scaffold::render_extra_deps,
    scaffold::scaffold_meta,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// The path (relative to the project root) of the Ruby native crate's Cargo.toml.
///
/// Single source of truth for where `scaffold_ruby_cargo` writes that manifest, so a caller
/// wanting to read it back (e.g. `MagnusBackend::generate_bindings`, cross-checking the manifest
/// against `codegen::cfg::collect_cfg_features`) does not have to re-derive the formula from an
/// unrelated path such as `alef build`'s own `lib.rs` output directory -- which a `[crates.output]
/// ruby = "..."` override can point somewhere this formula's `native/` segment does not appear
/// under at all. ~keep
pub(crate) fn ruby_native_manifest_path(config: &ResolvedCrateConfig) -> PathBuf {
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Ruby);
    PathBuf::from(format!(
        "{pkg_dir}/ext/{}_rb/native/Cargo.toml",
        core_crate_dir.replace('-', "_")
    ))
}

/// The core-crate dependency's `, features = [...]` suffix for the Ruby native crate, with any
/// name in `excluded_default_features` dropped.
///
/// Mirrors [`crate::scaffold::core_dep_features`] but additionally filters `[crates.ruby].features`
/// (or the crate-level `[crate] features` fallback) against `excluded_default_features` -- the
/// consumer-facing exclusion is meant to keep a feature off this dependency edge entirely, not just
/// out of the wrapper's own `default = [...]` array (see [`RubyConfig::excluded_default_features`]).
/// A backend-local filter rather than a change to the shared helper: every other caller of
/// `core_dep_features` has no exclusion knob to honour, so folding this in there would give it a
/// second, Ruby-only reason to change. ~keep
fn ruby_core_dep_features(config: &ResolvedCrateConfig, excluded: &HashSet<&str>) -> String {
    let features: Vec<&str> = config
        .features_for_language(Language::Ruby)
        .iter()
        .map(String::as_str)
        .filter(|f| !excluded.contains(f))
        .collect();
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

pub(crate) fn scaffold_ruby_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Ruby);
    let native_crate_dir = format!("{pkg_dir}/ext/{}_rb/native", core_crate_dir.replace('-', "_"));
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &native_crate_dir);
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

    let excluded_default_features: HashSet<&str> = config
        .ruby
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let features_str = ruby_core_dep_features(config, &excluded_default_features);
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
        format!("rb-sys = \"{}\"", tv::cargo::RB_SYS),
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
    if has_trait_bridges && !dep_lines.iter().any(|l| l.starts_with("tracing")) {
        dep_lines.push(format!("tracing = \"{}\"", tv::cargo::TRACING));
    }
    if has_streaming_adapter && !dep_lines.iter().any(|l| l.starts_with("futures")) {
        dep_lines.push("futures = \"0.3\"".to_owned());
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
    if !core_dep_line.is_empty() {
        dep_lines.push(core_dep_line);
    }
    crate::scaffold::sort_dependency_lines(&mut dep_lines);
    let deps_section = dep_lines.join("\n");

    let mut machete_ignored: Vec<&str> = vec!["rb-sys"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tracing");
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
    let mut cfg_features = crate::codegen::cfg::collect_cfg_features(api);
    // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
    // still get a forwarding entry below -- alef-task #374, regression in `ruby/tests.rs`. ~keep
    cfg_features.extend(excluded_default_features.iter().map(|name| (*name).to_string()));
    // A name in `excluded_default_features` is still declared below (so `cargo build --features
    // <name>` keeps working) but dropped from `default`, matching
    // `SwiftConfig::excluded_default_features`. ~keep
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        let lines = crate::codegen::cfg::cfg_default_and_forwarding_lines(
            &cfg_features,
            &config.name,
            &excluded_default_features,
        );
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    let lints_section = crate::scaffold::cargo_lints_section(config);
    let content = format!(
        r#"{pkg_header}

{machete_section}[lib]
name = "{lib_name}"
path = "../src/lib.rs"
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_section}
{core_target_blocks_section}{lints_section}"#,
        pkg_header = pkg_header,
        lints_section = lints_section,
        machete_section = machete_section,
        lib_name = lib_name,
        features_table = features_table,
        deps_section = deps_section,
        core_target_blocks_section = core_target_blocks_section,
    );

    Ok(vec![GeneratedFile {
        path: ruby_native_manifest_path(config),
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
    let required_ruby_version = config
        .ruby
        .as_ref()
        .and_then(|c| c.required_ruby_version.clone())
        .unwrap_or_else(|| ">= 3.2.0".to_string());

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
  spec.required_ruby_version = "{required_ruby_version}"
{metadata}  spec.metadata["rubygems_mfa_required"] = "true"

  candidate_files    = Dir.glob(%w[README* LICENSE* lib/**/* ext/**/* sig/**/* Steepfile]).select {{ |f| File.file?(f) }}
  spec.files         = candidate_files.grep_v(%r{{/(?:target|tmp)/|\.(?:bundle|so|dylib|dll|o|a|log)\z|\.dSYM/}})
  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/{ext_name}/native/extconf.rb"]

  spec.add_dependency "rb_sys", {rb_sys}
  spec.add_dependency "sorbet-runtime", "{sorbet_runtime}"
end
"#,
        gem_name = gem_name,
        ext_name = ext_name,
        version = version,
        required_ruby_version = required_ruby_version,
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
    # alef also generates the gemspec and Rakefile below; excluding them here means a
    # future RuboCop cop cannot deadlock generation the way Style/SelectByRegexp once did
    # against the gemspec (both files are alef-owned and rewritten on every `alef build`,
    # so the consumer has no way to hand-fix a violation in them).
    - "*.gemspec"
    - "Rakefile"

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

    // Ruby cross-compile platforms, each paired with the Rust target triple that ~keep
    // backs it. A platform whose triple is disabled via the workspace `[targets]` ~keep
    // opt-out table is dropped from the generated `CROSS_PLATFORMS` list. ~keep
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
  # where the method does not exist (older rb_sys lines predating the 0.9.130 floor).
  # rb-sys-dock 0.9.x ships images for Ruby 3.2, 3.3, 3.4, and 3.5; this list must
  # match those available images. Per-ABI platform gem windows are controlled by rake-compiler-dock.
  ext.cross_compile_versions = %w[3.5.0 3.4.9 3.3.11 3.2.11] if ext.respond_to?(:cross_compile_versions=)
end

RSpec::Core::RakeTask.new(:spec)

# rake-compiler's `compile` task is a no-op when cross_compile is true; the real
# work hangs off `compile:<ruby_platform>`. Wire `compile` → `compile:ruby` so
# both the dev shorthand and CI's
# `BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rake compile` actually build.
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
        // Appended last, deliberately: `test_scaffold_ruby_production_features` in
        // `src/scaffold/tests/ffi_go_java_ruby.rs` asserts this vec's entries by index, so
        // inserting the seed next to the `Rakefile` it belongs with would renumber every
        // later assertion. Appending still shifts one: the `Language::Ruby` scaffold arm
        // extends `scaffold_ruby_cargo`'s manifest onto the tail, so that manifest moves from
        // index 6 to 7 while every entry before the seed keeps its position.
        //
        // The `Rakefile` above unconditionally wires `RSpec::Core::RakeTask.new(:spec)` and
        // `task default: :spec`, so `rake` has always *run* a spec suite — against a `spec/`
        // directory nothing ever created. `rake spec` over an empty suite exits 0, so the
        // lane reported green while proving nothing; this seeds one real example so the
        // wiring has something to execute from day one. ~keep
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/spec/{gem_name_snake}_spec.rb")),
            content: scaffold_ruby_spec(api, config, &gem_name_snake),
            generated_header: false,
        },
    ])
}

/// Literal value the seed passes for a String field, and asserts back out of the accessor.
const RUBY_SEED_STRING_LITERAL: &str = "alef-scaffold";

/// Build the seed content for `{pkg_dir}/spec/{gem_name_snake}_spec.rb`.
///
/// `write_scaffold_files_report` skips any `generated_header: false` path that already
/// exists (`can_skip`), so this only ever seeds a fresh project and never overwrites a real
/// suite. Because the path is *new*, existing repos pick the seed up with no migration at all
/// — unlike the zig/dart/swift seeds, whose files already existed with the wrong content and
/// each needed an in-place repair pass to reach a repo that had already been generated once.
///
/// "Pick it up" means the next run of a command that actually writes scaffold output: `alef
/// scaffold`, `alef all`, or `alef init`. Plain `alef generate` does not — it calls
/// `pipeline::scaffold` only to feed `reconcile_managed_scaffold_manifests`, which filters the
/// set down to `generated_header: true` `.toml` manifests and drops every seed on the floor.
/// That is pre-existing behaviour for every scaffold seed, not something this path introduces,
/// but it is the difference between "next generate" and "next scaffold" and is worth knowing
/// before wondering why the file has not appeared. ~keep
///
/// The seed must not be vacuous: `expect(1).to eq(1)` passes no matter what alef generated,
/// which is strictly worse than an empty lane because it manufactures confidence. Every tier
/// below therefore asserts against the *real*, currently-generated API surface, and every
/// tier — including the weakest — goes through `require_relative "../lib/<gem>"`, which loads
/// `lib/<gem>/native.rb`; that file raises `LoadError` when the compiled extension is absent.
/// So no tier can pass without the native extension actually being built and dlopened. That
/// is the deliberate difference from `scaffold_zig_test`'s `@hasDecl` tier, which is
/// comptime-only and therefore never links or invokes anything (alef task #85).
///
/// Tiers, strongest first:
///
/// 1. A visible zero-parameter, non-async function with a primitive/`String` return whose
///    generated wrapper really delegates to the core crate is actually **called** through the
///    Magnus boundary and its result type-checked; an infallible one is preferred over a
///    fallible one. Proves: extension links, the module function is registered under that
///    name, and its return value converts to the mapped Ruby type. Does not prove: anything
///    about the value's semantics — the seed cannot know what the function should return.
/// 2. Otherwise a visible DTO whose every binding-visible field is a plain
///    primitive/`String` is **constructed** through the generated kwargs constructor and
///    every field read back through its generated accessor. Proves: the class is registered,
///    the constructor accepts those keyword symbols, and each accessor round-trips the value
///    it was given (a renamed or dropped field fails, because the constructor silently
///    ignores unknown keys and the accessor would return the default instead). Does not
///    prove: any behaviour beyond field storage.
/// 3. Otherwise a visible type name is resolved as a constant on the module. Proves: the
///    extension loaded and registered that class. Does not prove: its shape or that anything
///    can be called on it.
/// 4. Only when no visible function or type exists at all (scaffolding before any Rust code)
///    does this fall back to asserting `VERSION` — always emitted by `generate_public_api` —
///    has a version-like shape. Proves: the gem and the native extension both load. Does not
///    prove: any generated API exists, because at this point none does.
///
/// Enums are deliberately absent from the ladder: the Magnus backend does not register them
/// as Ruby constants (see the note above `public_types` in `MagnusBackend::generate_public_api`),
/// so a `{module_name}::SomeEnum` reference would name something that does not exist. ~keep
fn scaffold_ruby_spec(api: &ApiSurface, config: &ResolvedCrateConfig, gem_name_snake: &str) -> String {
    use heck::ToUpperCamelCase as _;

    // `get_module_name(&config.ruby_gem_name())` in `MagnusBackend::generate_public_api` — the
    // module the generated `lib/<gem>.rb` opens and the native extension registers into. ~keep
    let module_name = config.ruby_gem_name().to_upper_camel_case();
    let (exclude_functions, exclude_types) = ruby_binding_exclusions(api, config);

    let call_candidates: Vec<(&FunctionDef, &'static str)> = api
        .functions
        .iter()
        .filter(|f| ruby_function_is_callable_seed_target(f, config, &exclude_functions))
        .filter_map(|f| ruby_return_expectation(&f.return_type).map(|expectation| (f, expectation)))
        .collect();
    // Infallible first: a fallible function's generated wrapper propagates a core `Err` as a
    // raised Ruby exception, and a function that legitimately fails in a clean checkout (no
    // network, no model downloaded, no GPU) would make the seed permanently red on a healthy
    // build — a false alarm is only marginally better than the vacuous pass this replaces. A
    // fallible function is still used when it is the only candidate: an example that can fail
    // for a real reason beats degrading to a weaker tier. ~keep
    let call_candidate = call_candidates
        .iter()
        .find(|(f, _)| f.error_type.is_none())
        .or_else(|| call_candidates.first())
        .copied();
    if let Some((f, expectation)) = call_candidate {
        return ruby_spec(gem_name_snake, &module_name, &ruby_call_example(&f.name, expectation));
    }

    let construct_candidate = api
        .types
        .iter()
        .filter(|t| ruby_type_is_visible(t, &exclude_types))
        .find_map(|t| simple_ruby_fields(t).map(|fields| (t, fields)));
    if let Some((ty, fields)) = construct_candidate {
        return ruby_spec(gem_name_snake, &module_name, &ruby_construct_example(&ty.name, &fields));
    }

    if let Some(ty) = api.types.iter().find(|t| ruby_type_is_visible(t, &exclude_types)) {
        return ruby_spec(gem_name_snake, &module_name, &ruby_constant_example(&ty.name));
    }

    ruby_spec(gem_name_snake, &module_name, &ruby_version_example())
}

/// Names excluded from Ruby binding generation, mirroring the union `MagnusBackend` itself
/// computes (`src/backends/magnus/gen_bindings/mod.rs`): `[crates.ruby] exclude_functions` /
/// `exclude_types`, plus any type marked `binding_excluded`. Unlike the zig and dart seeds
/// this deliberately does **not** fold in `[crates.ffi]`'s lists — the Magnus backend reads
/// only `config.ruby`, and mirroring an exclusion it does not honour would make the seed skip
/// a name that really is emitted. `is_reserved_fn` is not mirrored either: its backing list
/// (`MAGNUS_RESERVED_FN_NAMES`) is currently empty, so mirroring it would be dead code.
fn ruby_binding_exclusions(api: &ApiSurface, config: &ResolvedCrateConfig) -> (HashSet<String>, HashSet<String>) {
    let exclude_functions: HashSet<String> = config
        .ruby
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut exclude_types: HashSet<String> = config
        .ruby
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    (exclude_functions, exclude_types)
}

/// A type the seed may safely name, mirroring the `public_types` filter in
/// `MagnusBackend::generate_public_api` — that curated list is what a gem whose module name
/// differs from its crate name re-exports, so anything outside it may be absent from the
/// module the spec describes. `cfg`-gated types are additionally skipped: the Magnus backend
/// prepends `#[cfg(...)]` to them, so whether they are registered depends on which features
/// the extension was compiled with, which this scaffold-time seed cannot know.
fn ruby_type_is_visible(ty: &TypeDef, exclude_types: &HashSet<String>) -> bool {
    !ty.is_trait
        && !ty.is_opaque
        && !ty.binding_excluded
        && ty.cfg.is_none()
        && !exclude_types.contains(&ty.name)
        && !ty.name.ends_with("Update")
        && !ty.name.ends_with("Builder")
}

/// Whether `f` is safe for the strongest tier: a zero-argument, non-async, non-`cfg`-gated
/// function the seed can call with no knowledge of any parameter's ownership or conversion
/// requirements. The return type is checked separately by the caller, via
/// [`ruby_return_expectation`].
///
/// Async functions are excluded even when they take no arguments: Magnus registers them under
/// a different Rust body (`ruby_native_function_name` appends `_async`) that drives a Tokio
/// runtime, which is not something a scaffold seed should be the first thing to exercise.
/// Trait-bridge-managed functions are excluded because `module_init` skips registering them
/// outright. Functions whose Ruby-visible name (`ruby_public_function_name`, the leaf of
/// `original_rust_path`) differs from `name` are excluded too: the native extension registers
/// the leaf while `generate_public_api`'s re-export list uses `name`, so only functions where
/// the two agree are reachable under one name in both layouts.
///
/// The delegability check is the load-bearing one, not a formality. When
/// [`crate::codegen::shared::can_auto_delegate_function`] is false, the Magnus wrapper
/// generator (`gen_magnus_unimplemented_body`) emits a body that *raises* `RuntimeError:
/// Not implemented: <name>` for a fallible function — the symbol is registered and callable,
/// so nothing here would notice, and the seed would be permanently red on a perfectly healthy
/// build. Passing an empty opaque-type set is exact rather than approximate: every remaining
/// term of that predicate ranges over `params`, which this function has already constrained to
/// be empty, leaving only `!sanitized` and the return type. ~keep
fn ruby_function_is_callable_seed_target(
    f: &FunctionDef,
    config: &ResolvedCrateConfig,
    exclude_functions: &HashSet<String>,
) -> bool {
    !f.binding_excluded
        && !exclude_functions.contains(&f.name)
        && f.cfg.is_none()
        && !f.is_async
        && f.params.is_empty()
        && !f.return_sanitized
        && crate::codegen::shared::can_auto_delegate_function(f, &ahash::AHashSet::default())
        && crate::backends::magnus::ruby_public_function_name(f) == f.name
        && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
}

/// The RSpec matcher asserting that a returned value has the Ruby type the Magnus type map
/// produces for `ty`, or `None` when the return type is not one this seed can type-check.
fn ruby_return_expectation(ty: &TypeRef) -> Option<&'static str> {
    match ty {
        TypeRef::String => Some("be_a(String)"),
        TypeRef::Primitive(primitive) => Some(ruby_primitive_expectation(primitive)),
        _ => None,
    }
}

fn ruby_primitive_expectation(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Bool => "be(true).or(be(false))",
        PrimitiveType::F32 | PrimitiveType::F64 => "be_a(Float)",
        _ => "be_a(Integer)",
    }
}

/// A field simple enough for the seed to both pass as a constructor keyword and assert back
/// out of its accessor.
struct SimpleRubyField {
    name: String,
    literal: String,
    /// Whether `literal` is a quoted Ruby string literal, as opposed to a bare `true`/numeric
    /// one -- read by [`ruby_construct_example`] to decide whether a multi-field literal array
    /// is all-String and therefore a `Style/WordArray` candidate.
    is_string: bool,
}

/// Compute a literal-constructible field list for `ty`, or `None` when any binding-visible
/// field falls outside the safely synthesizable subset. Bails on the *whole type* rather than
/// constructing it partially: a `Named` field with no default makes the generated constructor
/// raise `ArgumentError`, so a partial construction would fail at runtime rather than assert
/// anything.
///
/// Rejected: optional fields (the accessor returns `nil` semantics this seed would have to
/// model), `cfg`-gated fields, `binding_excluded` fields (dropped from the generated struct),
/// and anything whose name is not plain snake_case — the Ruby accessor and the constructor's
/// keyword symbol are both the raw Rust field name, so a positional name like `_0` would
/// produce an example naming a method that reads nothing.
fn simple_ruby_fields(ty: &TypeDef) -> Option<Vec<SimpleRubyField>> {
    if ty.has_stripped_cfg_fields {
        return None;
    }
    let mut fields = Vec::new();
    for field in crate::codegen::shared::binding_fields(&ty.fields) {
        if field.optional || field.cfg.is_some() || !is_plain_ruby_field_name(field) {
            return None;
        }
        let (literal, is_string) = match &field.ty {
            TypeRef::Primitive(primitive) => (ruby_primitive_literal(primitive).to_string(), false),
            TypeRef::String => (format!("\"{RUBY_SEED_STRING_LITERAL}\""), true),
            _ => return None,
        };
        fields.push(SimpleRubyField {
            name: field.name.clone(),
            literal,
            is_string,
        });
    }
    if fields.is_empty() { None } else { Some(fields) }
}

fn is_plain_ruby_field_name(field: &FieldDef) -> bool {
    let mut chars = field.name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A literal Ruby value for a primitive field. `bool` gets a non-default `true` and floats a
/// non-integral `1.5` so a constructor that silently drops the keyword and falls back to the
/// field's default is still caught by the accessor assertion.
fn ruby_primitive_literal(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Bool => "true",
        PrimitiveType::F32 | PrimitiveType::F64 => "1.5",
        _ => "1",
    }
}

/// Wrap one generated example in the spec file's frame. `require_relative` (rather than
/// `require`) is deliberate: it resolves against the package directory, so `rake spec` works
/// straight out of a fresh scaffold without a `spec_helper.rb` or a `$LOAD_PATH` entry.
fn ruby_spec(gem_name_snake: &str, module_name: &str, example: &str) -> String {
    format!(
        r#"# frozen_string_literal: true

require_relative "../lib/{gem_name_snake}"

RSpec.describe {module_name} do
{example}end
"#
    )
}

fn ruby_call_example(function_name: &str, expectation: &str) -> String {
    format!(
        r#"  # Calls the generated `{function_name}` module function end-to-end. The
  # `require_relative` above loads the gem, whose `native.rb` dlopens the compiled
  # extension and raises LoadError when it is missing, so this example crosses the real
  # Magnus boundary: it fails on an unbuilt extension, a link error, or a removed or
  # renamed export. It does not assert *what* the value should be -- only that the
  # binding returns a value of the mapped Ruby type. Create-only scaffold seed: alef never
  # regenerates over this file, so replace it with a real suite. ~keep
  it "calls the generated `{function_name}` module function" do
    expect(described_class.{function_name}).to({expectation})
  end
"#
    )
}

fn ruby_construct_example(type_name: &str, fields: &[SimpleRubyField]) -> String {
    let kwargs = fields
        .iter()
        .map(|f| format!("{}: {}", f.name, f.literal))
        .collect::<Vec<_>>()
        .join(", ");
    let assertion = if let [only] = fields {
        format!(
            "    expect(instance.{name}).to(eq({literal}))",
            name = only.name,
            literal = only.literal
        )
    } else {
        // One expectation over every field rather than one per field: the generated
        // `.rubocop.yml` caps `RSpec/MultipleExpectations` at 25, and a DTO can carry more
        // fields than that. ~keep
        let readers = fields
            .iter()
            .map(|f| format!("instance.{}", f.name))
            .collect::<Vec<_>>()
            .join(", ");
        // An all-String field list repeats the one literal `RUBY_SEED_STRING_LITERAL` (a
        // hyphenated word) two or more times, which is exactly what `Style/WordArray` flags in
        // the `.rubocop.yml` scaffolded next to this file -- its `WordRegex` explicitly permits
        // one hyphen, and its default `MinSize` is 2. `%w[]` is semantically identical for a
        // literal, non-interpolated word with no special characters and sidesteps the cop
        // entirely; a mixed-type field list (bool/numeric literals are unquoted, so the array is
        // never all-String) keeps the bracket form, which the cop does not flag. ~keep
        let literals_array = if fields.iter().all(|f| f.is_string) {
            format!("%w[{}]", vec![RUBY_SEED_STRING_LITERAL; fields.len()].join(" "))
        } else {
            let literals = fields.iter().map(|f| f.literal.clone()).collect::<Vec<_>>().join(", ");
            format!("[{literals}]")
        };
        format!("    expect([{readers}]).to(eq({literals_array}))")
    };
    format!(
        r#"  # No generated function is safe to call with no arguments, so this exercises the
  # binding through the generated `{type_name}` class instead: the `require_relative`
  # above dlopens the compiled extension (LoadError when missing), the keyword
  # constructor registered by Magnus is invoked, and every field is read back through its
  # generated accessor. A dropped or renamed field fails here, because the constructor
  # ignores unknown keys and the accessor would return the field's default instead of the
  # value passed in. It proves nothing beyond field storage. Create-only scaffold seed:
  # alef never regenerates over this file, so replace it with a real suite. ~keep
  it "constructs the generated `{type_name}` class from keyword arguments" do
    instance = described_class::{type_name}.new({kwargs})
{assertion}
  end
"#
    )
}

fn ruby_constant_example(type_name: &str) -> String {
    format!(
        r#"  # `{type_name}` is not literal-constructible by a seed that cannot synthesize values
  # for its fields, so this only resolves it as a constant on the module. The
  # `require_relative` above still dlopens the compiled extension (LoadError when
  # missing) and the constant only exists because the extension registered the class, so
  # this fails on an unbuilt extension or a removed type -- but it proves nothing about
  # the class's shape and calls nothing on it. Create-only scaffold seed: alef never
  # regenerates over this file, so replace it with a real suite. ~keep
  it "registers the generated `{type_name}` class on the module" do
    expect(described_class.const_get(:{type_name})).to(be_a(Module))
  end
"#
    )
}

fn ruby_version_example() -> String {
    r#"  # No generated API surface exists yet for this crate, so there is nothing to assert
  # against beyond the gem loading. `VERSION` is emitted unconditionally by alef, and the
  # `require_relative` above pulls in `native.rb`, which raises LoadError when the compiled
  # extension is missing -- so this still fails on an unbuilt or unlinkable extension. It
  # proves no generated API exists, because at this point none does. The version is matched
  # by shape, not value, because this file is a create-only scaffold seed alef never
  # regenerates over -- pinning the exact version would break on the next release. ~keep
  it "loads the native extension and exposes a version" do
    expect(described_class::VERSION).to match(/\A\d+\.\d+\.\d+/)
  end
"#
    .to_string()
}

#[cfg(test)]
#[path = "ruby/tests.rs"]
mod tests;
