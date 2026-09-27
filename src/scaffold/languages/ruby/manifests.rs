use crate::core::config::extras::Language;
use crate::scaffold::ScaffoldMeta;
use crate::scaffold::dependency_versions::{ManagedVersions, override_notice_hash};

/// Everything [`ruby_gemspec_content`] needs from `super::scaffold_ruby`, which derives each
/// field from the same `ResolvedCrateConfig`/`ApiSurface` pair it uses for the package's other
/// scaffolded files.
pub(super) struct RubyGemspecParams<'a> {
    pub meta: &'a ScaffoldMeta,
    pub gem_name: &'a str,
    pub gem_name_snake: &'a str,
    pub ext_name: &'a str,
    pub version: &'a str,
    pub required_ruby_version: &'a str,
    pub versions: &'a ManagedVersions<'a>,
}

fn ruby_authors_literal(meta: &ScaffoldMeta) -> String {
    if meta.authors.is_empty() {
        "[]".to_string()
    } else {
        let entries: Vec<String> = meta.authors.iter().map(|a| format!("\"{}\"", a)).collect();
        format!("[{}]", entries.join(", "))
    }
}

fn ruby_keywords_metadata(meta: &ScaffoldMeta) -> String {
    if meta.keywords.is_empty() {
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
    }
}

fn ruby_homepage_line(meta: &ScaffoldMeta) -> String {
    meta.configured_repository
        .as_deref()
        .map(|repository| format!("  spec.homepage      = \"{repository}\"\n"))
        .unwrap_or_default()
}

fn ruby_license_lines(meta: &ScaffoldMeta) -> String {
    meta.license
        .as_deref()
        .map(|license| {
            let licenses: Vec<&str> = license
                .split(" OR ")
                .map(str::trim)
                .filter(|license| !license.is_empty())
                .collect();
            if licenses.len() > 1 {
                let values = licenses
                    .iter()
                    .map(|license| format!("\"{license}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("  spec.licenses      = [{values}]\n")
            } else {
                format!("  spec.license       = \"{license}\"\n")
            }
        })
        .unwrap_or_default()
}

pub(super) fn ruby_gemspec_content(params: RubyGemspecParams<'_>) -> String {
    let RubyGemspecParams {
        meta,
        gem_name,
        gem_name_snake,
        ext_name,
        version,
        required_ruby_version,
        versions,
    } = params;
    let authors_ruby = ruby_authors_literal(meta);
    let metadata_ruby = ruby_keywords_metadata(meta);
    let homepage_ruby = ruby_homepage_line(meta);
    let license_ruby = ruby_license_lines(meta);

    format!(
        r#"# frozen_string_literal: true
{version_notice}
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

  # Keep sibling packages and retained legacy inputs out of this gem's archive.
  candidate_files    = Dir.glob(%W[README* LICENSE* lib/{gem_name_snake}.rb lib/{gem_name_snake}/**/* ext/{ext_name}/**/* sig/**/* Steepfile]).select {{ |f| File.file?(f) }}
  spec.files         = candidate_files.grep_v(%r{{/(?:target|tmp)/|\.(?:bundle|so|dylib|dll|o|a|log)\z|\.dSYM/}})
  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/{ext_name}/native/extconf.rb"]

  spec.add_dependency "rb_sys", {rb_sys}
  spec.add_dependency "sorbet-runtime", "{sorbet_runtime}"
end
"#,
        version_notice = override_notice_hash(Language::Ruby),
        gem_name = gem_name,
        ext_name = ext_name,
        version = version,
        required_ruby_version = required_ruby_version,
        authors = authors_ruby,
        description = meta.description,
        homepage = homepage_ruby,
        license = license_ruby,
        metadata = metadata_ruby,
        rb_sys = versions.get("gem:rb_sys"),
        sorbet_runtime = versions.get("gem:sorbet-runtime"),
    )
}

pub(super) fn ruby_gemfile_content(versions: &ManagedVersions<'_>) -> String {
    format!(
        r#"# frozen_string_literal: true
{version_notice}
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
        version_notice = override_notice_hash(Language::Ruby),
        rake_compiler = versions.get("gem:rake-compiler"),
        rb_sys = versions.get("gem:rb_sys"),
        rspec = versions.get("gem:rspec"),
        rubocop = versions.get("gem:rubocop"),
        rubocop_performance = versions.get("gem:rubocop-performance"),
        rubocop_rspec = versions.get("gem:rubocop-rspec"),
        steep = versions.get("gem:steep"),
    )
}
