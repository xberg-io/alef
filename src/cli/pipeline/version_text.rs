use crate::core::config::{CitationAuthor, CitationConfig};
use std::sync::LazyLock;
use tracing::debug;

/// Update all `<gem-name> (<old-version>)` entries in a Gemfile.lock to `new_ruby_version`.
///
/// Gemfile.lock records the path-gem version in two places:
///
/// 1. Under `PATH > specs:` — four-space indent, may include dependency lines below it.
/// 2. Under `CHECKSUMS` — two-space indent, no sha256 suffix (path gems are not downloaded).
///
/// Both patterns look like `  <name> (<version>)` with varying indentation. We replace
/// every occurrence of `<name> (<old>)` with `<name> (<new>)` regardless of indent, so
/// the function handles any future Gemfile.lock layout changes automatically.
///
/// Returns `Some(new_content)` when at least one substitution was made, `None` when the
/// lockfile already contains the target version everywhere (idempotent).
pub(super) fn sync_gemfile_lock(content: &str, new_ruby_version: &str) -> Option<String> {
    use std::sync::LazyLock;
    static GEM_VERSION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^([ \t]*)([A-Za-z0-9_-]+) \(([^)]+)\)$").expect("valid regex"));

    let path_gem_names: std::collections::HashSet<String> = {
        let mut names = std::collections::HashSet::new();
        let mut in_specs = false;
        for line in content.lines() {
            line.trim_start().starts_with("specs:");
            if line == "PATH" {
                in_specs = true;
                continue;
            }
            if in_specs && line.starts_with("  specs:") {
                continue;
            }
            if in_specs && line.starts_with("    ") {
                if let Some(caps) = GEM_VERSION_RE.captures(line) {
                    let indent = &caps[1];
                    let name = &caps[2];
                    if indent.len() == 4 {
                        names.insert(name.to_string());
                    }
                }
                continue;
            }
            if in_specs
                && !line.starts_with("    ")
                && !line.trim().is_empty()
                && line != "PATH"
                && !line.starts_with("  ")
            {
                in_specs = false;
            }
        }
        names
    };

    if path_gem_names.is_empty() {
        return None;
    }

    let mut changed = false;
    let new_content = content
        .lines()
        .map(|line| {
            if let Some(caps) = GEM_VERSION_RE.captures(line) {
                let gem_name = &caps[2];
                let current_version = &caps[3];
                if path_gem_names.contains(gem_name) && current_version != new_ruby_version {
                    changed = true;
                    let indent = &caps[1];
                    return format!("{indent}{gem_name} ({new_ruby_version})");
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };

    if changed { Some(new_content) } else { None }
}

/// Rewrite the dependency `<version>` and `<systemPath>` in an e2e `pom.xml`
/// for a path-scope system dependency on the library JAR.
///
/// The e2e `pom.xml` carries a `<dependency>` block like:
/// ```xml
/// <dependency>
///   <groupId>dev.sample_core.sample_widget</groupId>
///   <artifactId>sample-widget</artifactId>
///   <version>0.3.0-rc.27</version>
///   <scope>system</scope>
///   <systemPath>.../sample-widget-0.3.0-rc.27.jar</systemPath>
/// </dependency>
/// ```
/// Unlike `packages/java/pom.xml`, this file has a *separate* `<version>0.1.0</version>`
/// for the e2e project itself at the top — we must not touch that one.
///
/// Strategy: two passes.
///
/// 1. Collect the byte-ranges of every `<dependency>...</dependency>` block
///    that contains a `<systemPath>` element.
/// 2. Within those ranges, rewrite `<version>X</version>` and the version
///    fragment inside `<systemPath>`.
///
/// All other `<version>` tags are left untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
pub(super) fn sync_e2e_java_pom(content: &str, new_version: &str) -> Option<String> {
    use std::sync::LazyLock;

    static DEP_BLOCK_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)<dependency>(.*?)</dependency>").expect("valid regex"));
    static VERSION_TAG_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<version>([^<]*)</version>").expect("valid regex"));
    static SYSTEM_PATH_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(<systemPath>[^<]*?-)(\d+\.\d+\.\d+(?:-[A-Za-z0-9._]+)*)(\.[a-zA-Z]+</systemPath>)")
            .expect("valid regex")
    });

    let mut result = content.to_string();
    let mut changed = false;

    let dep_matches: Vec<(usize, usize, String)> = DEP_BLOCK_RE
        .find_iter(content)
        .filter_map(|m| {
            let block = m.as_str();
            if !block.contains("<systemPath>") {
                return None;
            }
            let new_block = VERSION_TAG_RE
                .replace(block, |caps: &regex::Captures<'_>| {
                    let ver = &caps[1];
                    if ver != new_version && !ver.contains('$') && !ver.contains('.') && ver.parse::<u64>().is_err() {
                        format!("<version>{ver}</version>")
                    } else if ver != new_version && ver.contains('.') && !ver.contains('$') {
                        format!("<version>{new_version}</version>")
                    } else {
                        format!("<version>{ver}</version>")
                    }
                })
                .into_owned();
            let new_block = SYSTEM_PATH_RE
                .replace(&new_block, |caps: &regex::Captures<'_>| {
                    format!("{}{}{}", &caps[1], new_version, &caps[3])
                })
                .into_owned();
            if new_block != block {
                Some((m.start(), m.end(), new_block))
            } else {
                None
            }
        })
        .collect();

    for (start, end, new_block) in dep_matches.into_iter().rev() {
        result.replace_range(start..end, &new_block);
        changed = true;
    }

    if changed { Some(result) } else { None }
}

/// Rewrite the version for a module in a `go.mod` `require` block.
///
/// The e2e `go.mod` has a line like:
/// ```text
/// github.com/sample-core-dev/sample-widget/packages/go v0.3.0-rc.27
/// ```
/// We want to update ONLY lines whose module path matches `module_path_fragment`
/// — a substring that uniquely identifies the library module (e.g.
/// `"sample-core-dev/sample-widget/packages/go"`). All other `require` entries are
/// left untouched.
///
/// When the same module is the target of a local `replace` directive
/// (`require ... => ../../packages/go`), Go ignores the `require` version and
/// the `alef all` generate path emits the conventional placeholder `v0.0.0`.
/// Bumping the require line in that case produces drift versus the generated
/// output, so this function leaves a locally-replaced module untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
pub(super) fn sync_e2e_go_mod(content: &str, module_path_fragment: &str, new_version: &str) -> Option<String> {
    let has_local_replace = content.lines().any(|line| {
        let trimmed = line.trim_start();
        let trimmed = trimmed.strip_prefix("replace ").unwrap_or(trimmed);
        if let Some((lhs, rhs)) = trimmed.split_once("=>") {
            lhs.trim().starts_with(module_path_fragment) && {
                let dest = rhs.trim();
                dest.starts_with("./") || dest.starts_with("../") || dest.starts_with('/')
            }
        } else {
            false
        }
    });
    if has_local_replace {
        return None;
    }

    let mut changed = false;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(module_path_fragment) || line.trim_start().starts_with(module_path_fragment) {
                if let Some(pos) = trimmed.rfind(" v") {
                    let current_ver = &trimmed[pos + 2..];
                    if current_ver != new_version {
                        changed = true;
                        let indent = &line[..line.len() - line.trim_start().len()];
                        let module_path = &trimmed[..pos];
                        return format!("{indent}{module_path} v{new_version}");
                    }
                }
            }
            line.to_string()
        })
        .collect();

    if !changed {
        return None;
    }
    let new_content = lines.join("\n");
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };
    Some(new_content)
}

/// Rewrite the `from:` version bound on the *first-party* SwiftPM dependency in
/// a generated `Package.swift`, leaving external `.package(url:..., from:...)`
/// dependencies untouched.
///
/// A generated test_apps / e2e `Package.swift` may declare several SwiftPM
/// dependencies, only one of which is the consumer's own published package, e.g.
/// ```swift
/// .package(url: "https://github.com/example-org/example-swift-package", from: "1.10.2"),
/// .package(url: "https://github.com/tree-sitter/swift-tree-sitter", from: "0.25.0"),
/// ```
/// The naive `replace_version_pattern(content, "from:\\s*\"[^\"]*\"", version)`
/// rewrites the *first* `from:` it finds — which clobbers the external
/// `swift-tree-sitter` pin when that entry happens to appear first. Only the
/// first-party entry (the one whose URL matches `repo_url`, modulo a trailing
/// `.git` / `/`) tracks the workspace version.
///
/// Matching is done per `.package(url:...)` line: a line is first-party when its
/// quoted URL, normalised (strip scheme, trailing `.git`, trailing `/`), is a
/// prefix-or-equal match of the normalised `repo_url`. Returns `Some(new_content)`
/// when a first-party `from:` actually changed, `None` otherwise (idempotent).
pub(super) fn sync_swift_first_party_from(content: &str, repo_url: &str, new_version: &str) -> Option<String> {
    use std::sync::LazyLock;
    static PACKAGE_URL_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"\.package\(\s*url:\s*"([^"]*)"\s*,\s*from:\s*"([^"]*)""#).expect("valid regex")
    });

    let target = normalize_repo_url(repo_url);
    if target.is_empty() {
        return None;
    }

    let mut changed = false;
    let new_content = PACKAGE_URL_RE
        .replace_all(content, |caps: &regex::Captures<'_>| {
            let url = &caps[1];
            let current = &caps[2];
            let normalized = normalize_repo_url(url);
            let is_first_party = !normalized.is_empty() && repo_url_matches(&normalized, &target);
            if is_first_party && current != new_version {
                changed = true;
                format!(r#".package(url: "{url}", from: "{new_version}""#)
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    if changed { Some(new_content) } else { None }
}

/// Rewrite the version segment of the artifactbundle release-download URL in a
/// root `Package.swift`.
///
/// `scaffold_swift` emits the `.binaryTarget` URL with a
/// `releases/download/v__ALEF_SWIFT_VERSION__/…` placeholder that the first
/// `sync-versions` substitutes to a concrete `releases/download/vX.Y.Z/…`.  On
/// every *subsequent* bump the placeholder is already gone, so the plain
/// placeholder substitution is a no-op and the URL stays pinned at the
/// previously-released tag — a downstream consumer resolving `from: "X.Y.Z"`
/// then downloads the wrong (older) artifactbundle.  This helper rewrites the
/// concrete `releases/download/vX.Y.Z/` segment to `new_version` so the URL
/// always tracks the workspace version.  It mirrors the check that
/// `verify_versions` already performs against the exact same URL shape.
///
/// The regex is anchored to the literal `releases/download/v` prefix and the
/// trailing `/`, so unrelated `vX.Y.Z` text elsewhere in the manifest (target
/// names, comments) is never touched.  Returns `Some(new_content)` only when a
/// version segment actually changed — idempotent otherwise.
pub(super) fn sync_swift_binary_release_url(content: &str, new_version: &str) -> Option<String> {
    static RELEASE_URL_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(releases/download/v)(\d+\.\d+\.\d+(?:-[a-zA-Z0-9._]+)*)(/)").expect("valid regex")
    });
    let mut changed = false;
    let new_content = RELEASE_URL_RE
        .replace_all(content, |caps: &regex::Captures<'_>| {
            if &caps[2] != new_version {
                changed = true;
            }
            format!("{}{}{}", &caps[1], new_version, &caps[3])
        })
        .into_owned();
    if changed { Some(new_content) } else { None }
}

/// Normalise a repository URL for first-party comparison: drop the scheme, a
/// trailing `.git`, and any trailing slash so `https://host/org/repo`,
/// `https://host/org/repo.git`, and `host/org/repo/` all compare equal.
fn normalize_repo_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    after_scheme.trim_end_matches('/').trim_end_matches(".git").to_string()
}

fn repo_url_matches(candidate: &str, target: &str) -> bool {
    candidate == target
        || candidate
            .strip_prefix(target)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Rewrite the `version:` field for a path-source package in a Dart `pubspec.lock`.
///
/// Dart's pub lockfile has entries like:
/// ```yaml
///   sample-widget:
///     dependency: "direct main"
///     description:
///       path: "../../packages/dart"
///       relative: true
///     source: path
///     version: "0.3.0-rc.23"
/// ```
/// We match the package name, confirm it is a `source: path` entry, and rewrite
/// only its `version:` scalar. Registry (hosted) packages are left untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
pub(super) fn sync_e2e_dart_pubspec_lock(content: &str, new_version: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut result: Vec<String> = Vec::with_capacity(n);
    let mut changed = false;
    let mut i = 0;

    while i < n {
        let line = lines[i];
        if line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':') {
            let block_start = i;
            i += 1;
            while i < n && (lines[i].starts_with("    ") || lines[i].trim().is_empty()) {
                i += 1;
            }
            let block = &lines[block_start..i];

            let is_path_source = block.iter().any(|l| l.trim() == "source: path");
            if is_path_source {
                for &bline in block {
                    let trimmed = bline.trim();
                    if trimmed.starts_with("version:") {
                        let val = trimmed.trim_start_matches("version:").trim().trim_matches('"');
                        if val != new_version {
                            changed = true;
                            let indent = &bline[..bline.len() - bline.trim_start().len()];
                            result.push(format!("{indent}version: \"{new_version}\""));
                        } else {
                            result.push(bline.to_string());
                        }
                    } else {
                        result.push(bline.to_string());
                    }
                }
            } else {
                for &bline in block {
                    result.push(bline.to_string());
                }
            }
        } else {
            result.push(line.to_string());
            i += 1;
        }
    }

    if !changed {
        return None;
    }
    let new_content = result.join("\n");
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };
    Some(new_content)
}

/// Read the workspace license string (`[workspace.package].license`) from a
/// Cargo.toml path. Used as the fallback `license:` value for CITATION.cff
/// when the `[workspace.citation]` block omits it. Returns `None` on any
/// read/parse failure or when the field is absent — caller decides what to do.
pub(super) fn read_workspace_license(version_from: &str) -> Option<String> {
    let content = std::fs::read_to_string(version_from).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;
    value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("license"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("package")
                .and_then(|p| p.get("license"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

/// Render a full `CITATION.cff` YAML document from config + canonical version.
///
/// Emits fields in the canonical CFF order (`cff-version`, `message`, `title`,
/// `abstract`, `authors`, `repository-code`, `url`, `license`, `version`,
/// `date-released`, `doi`). Author entries are emitted as either person-form
/// (`family-names` + `given-names`) or entity-form (`name`) depending on which
/// fields are populated; if both styles are set on a single author the person
/// form wins. Strings containing characters that need escaping (`:`, `#`, `\`,
/// `"`) are emitted double-quoted; otherwise the renderer uses bare scalars.
///
/// `date-released` policy: `[workspace.citation].date-released` in `alef.toml`
/// is an optional override. When set, that exact value is emitted verbatim.
/// When omitted (the recommended default), the caller-supplied `today` date
/// is emitted instead — this lets `alef sync-versions` auto-stamp the current
/// system date on every regen, so consumers do not need to hand-edit alef.toml
/// per release. `today` must already be formatted as `YYYY-MM-DD` per the CFF
/// spec.
pub(super) fn render_citation_cff(
    citation: &CitationConfig,
    version: &str,
    fallback_license: Option<&str>,
    today: &str,
) -> String {
    let mut out = String::new();
    out.push_str("# This file is generated by alef sync-versions; do not edit by hand.\n");
    out.push_str("# Source: [workspace.citation] in alef.toml + workspace version in Cargo.toml.\n");
    out.push_str("cff-version: 1.2.0\n");
    out.push_str(&format!("message: {}\n", yaml_scalar(&citation.message)));
    out.push_str(&format!("title: {}\n", yaml_scalar(&citation.title)));
    out.push_str(&format!("abstract: {}\n", yaml_scalar(&citation.abstract_)));
    out.push_str("authors:\n");
    for author in &citation.authors {
        out.push_str(&render_citation_author(author));
    }
    out.push_str(&format!(
        "repository-code: {}\n",
        yaml_scalar(&citation.repository_code)
    ));
    if let Some(url) = &citation.url {
        out.push_str(&format!("url: {}\n", yaml_scalar(url)));
    }
    let license = citation.license.as_deref().or(fallback_license);
    if let Some(license) = license {
        out.push_str(&format!("license: {}\n", yaml_scalar(license)));
    }
    out.push_str(&format!("version: {version}\n"));
    let date_value = citation.date_released.as_deref().unwrap_or(today);
    out.push_str(&format!("date-released: {}\n", yaml_scalar(date_value)));
    if let Some(doi) = &citation.doi {
        out.push_str(&format!("doi: {}\n", yaml_scalar(doi)));
    }
    out
}

/// Render a single `authors:` list entry. Two-space indent (`  - key: value`)
/// matches the canonical CITATION.cff layout produced by `cffinit`.
fn render_citation_author(author: &CitationAuthor) -> String {
    let mut entry = String::new();
    let person_form = author.family_names.is_some() || author.given_names.is_some();
    if person_form {
        if let Some(family) = &author.family_names {
            entry.push_str(&format!("  - family-names: {}\n", yaml_scalar(family)));
            if let Some(given) = &author.given_names {
                entry.push_str(&format!("    given-names: {}\n", yaml_scalar(given)));
            }
        } else if let Some(given) = &author.given_names {
            entry.push_str(&format!("  - given-names: {}\n", yaml_scalar(given)));
        }
        if let Some(email) = &author.email {
            entry.push_str(&format!("    email: {}\n", yaml_scalar(email)));
        }
        if let Some(orcid) = &author.orcid {
            entry.push_str(&format!("    orcid: {}\n", yaml_scalar(orcid)));
        }
    } else if let Some(name) = &author.name {
        entry.push_str(&format!("  - name: {}\n", yaml_scalar(name)));
        if let Some(email) = &author.email {
            entry.push_str(&format!("    email: {}\n", yaml_scalar(email)));
        }
        if let Some(orcid) = &author.orcid {
            entry.push_str(&format!("    orcid: {}\n", yaml_scalar(orcid)));
        }
    }
    entry
}

/// Emit a YAML scalar — double-quoted with escaping when the value contains
/// characters that would change YAML parsing semantics (`:`, `#`, leading
/// special chars, embedded quotes), bare otherwise. Tuned for the limited set
/// of strings that appear in CITATION.cff (titles, names, URLs, abstracts).
pub(super) fn yaml_scalar(value: &str) -> String {
    let needs_quoting = value.is_empty()
        || value.contains(':')
        || value.contains('#')
        || value.contains('"')
        || value.contains('\\')
        || value.contains('\n')
        || value.contains('\t')
        || value.contains(' ')
        || value.contains('\'')
        || value.contains('@')
        || value.starts_with(['!', '&', '*', '?', '|', '>', '"', '%', '`', '[', ']', '{', '}', ',']);
    if needs_quoting {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// Regex for the top-level `version:` key in a CITATION.cff YAML file.
/// Anchored to start-of-line so nested `version:` keys inside `references:` /
/// `preferred-citation:` blocks (which are indented) are not touched.
/// The Rust `regex` crate has no backreferences, so each quote style is its
/// own alternation arm and the matching arm tells us which to emit back.
/// Capture groups:
///   1. literal `version:` + spacing
///   2. value when double-quoted
///   3. value when single-quoted
///   4. value when unquoted (bare scalar)
static CITATION_VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?m)^(version:[ \t]+)(?:"([^"\n]*)"|'([^'\n]*)'|([^\s#'"]+))[ \t]*(?:#[^\n]*)?$"#)
        .expect("valid regex")
});

/// Update the top-level `version:` scalar in a CITATION.cff. Preserves the
/// original quote style (unquoted, single-, or double-quoted). Returns
/// `Some(new_content)` only when the value actually changes — guards against
/// idempotent re-writes that would dirty the working tree on every sync.
pub(super) fn replace_citation_version(content: &str, new_version: &str) -> Option<String> {
    let captures = CITATION_VERSION_RE.captures(content)?;
    let (current, replacement) = if let Some(value) = captures.get(2) {
        (value.as_str(), format!("{}\"{new_version}\"", &captures[1]))
    } else if let Some(value) = captures.get(3) {
        (value.as_str(), format!("{}'{new_version}'", &captures[1]))
    } else if let Some(value) = captures.get(4) {
        (value.as_str(), format!("{}{new_version}", &captures[1]))
    } else {
        return None;
    };
    if current == new_version {
        return None;
    }
    let new_content = CITATION_VERSION_RE.replace(content, replacement.as_str()).into_owned();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Replace version pattern in content. Returns `Some(new_content)` only when
/// the regex match exists *and* the captured version string actually differs
/// from the target. This is the idempotency guard against:
///   1. backend codegen that emits a manifest with the right value but in a
///      slightly different syntactic form (e.g. Magnus emits `VERSION =
///      "4.10.0.pre.rc.9"` while the regex's replacement template uses
///      single-quotes); without this guard the two paths ping-pong and every
///      warm `alef generate` rewrites the manifest, triggers README regen,
///      and looks like real drift to downstream tooling.
///   2. trivial round-trips where new content == old content despite the
///      regex matching.
pub(super) fn replace_version_pattern(content: &str, pattern: &str, version: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    let captures = regex.captures(content)?;
    let matched = captures.get(0)?.as_str();
    if matched_version_equals(matched, version) {
        return None;
    }

    let replacement = match pattern {
        p if p.contains("version =") && !p.contains("spec") && !p.contains("VERSION") => {
            format!(r#"version = "{version}""#)
        }
        p if p.contains("\"version\"") && p.contains("\"") => format!(r#""version": "{version}""#),
        p if p.contains("spec") => format!("spec.version = \"{version}\""),
        p if p.contains("<version>") => format!("<version>{version}</version>"),
        p if p.contains("<InformationalVersion>") => {
            format!("<InformationalVersion>{version}</InformationalVersion>")
        }
        p if p.contains("<Version>") => format!("<Version>{version}</Version>"),
        p if p.contains("@version") => format!(r#"@version "{version}""#),
        p if p.contains("version:") && p.contains(":") => format!(r#"version: "{version}""#),
        p if p.contains("__version__") => format!(r#"__version__ = "{version}""#),
        p if p.contains("defaultFFIVersion") => format!(r#"defaultFFIVersion = "{version}""#),
        p if p.contains("moduleVersion") => format!(r#"moduleVersion = "{version}""#),
        p if p.contains("Version:") => format!("Version: {version}"),
        p if p.contains("from:") => format!(r#"from: "{version}""#),
        p if p.contains("VERSION=\"") => format!(r#"VERSION="{version}""#),
        p if p.contains("VERSION") => format!("VERSION = \"{version}\""),
        _ => return None,
    };

    let new_content = regex.replace(content, replacement.as_str()).to_string();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Extract the version-literal substring from a regex match string and decide
/// whether it already equals `target`. The match string is something like
/// `VERSION = "1.2.3"`, `version = "1.2.3"`, `<version>1.2.3</version>`,
/// `Version: 1.2.3`. We look for the first chunk after the delimiter and
/// compare it to `target`; quote style is irrelevant.
pub(super) fn matched_version_equals(matched: &str, target: &str) -> bool {
    extract_version_literal(matched).is_some_and(|v| v == target)
}

/// Restore canonical hex dependency version ranges in `gleam.toml`.
///
/// Earlier alef releases sometimes routed `gleam.toml` through the catch-all
/// `SEMVER_RE.replace_all` path, which rewrote every `\d+\.\d+\.\d+` literal
/// in the file with the workspace version — turning
/// `gleam_stdlib = ">= 0.34.0 and < 2.0.0"` into
/// `gleam_stdlib = ">= 5.0.0-rc.1 and < 5.0.0-rc.1"` (an empty version range
/// that gleam refuses to resolve).
///
/// This helper deterministically restores the canonical ranges from
/// `template_versions::hex` whenever it sees a `gleam_stdlib` or `gleeunit`
/// dependency line, so a single `alef sync-versions` heals affected
/// manifests without manual intervention.
pub(super) fn restore_gleam_dep_ranges(content: &str) -> String {
    use crate::core::template_versions::hex;
    static GLEAM_DEP_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"(?m)^(gleam_stdlib|gleeunit)\s*=\s*"([^"]*)""#).expect("valid regex"));

    GLEAM_DEP_RE
        .replace_all(content, |caps: &regex::Captures<'_>| {
            let name = &caps[1];
            let canonical = match name {
                "gleam_stdlib" => hex::GLEAM_STDLIB_VERSION_RANGE,
                "gleeunit" => hex::GLEEUNIT_VERSION_RANGE,
                _ => return caps[0].to_string(),
            };
            format!("{name} = \"{canonical}\"")
        })
        .into_owned()
}

fn extract_version_literal(matched: &str) -> Option<&str> {
    if let Some(start) = matched.find(['"', '\'']) {
        let quote = matched.as_bytes()[start];
        let rest = &matched[start + 1..];
        if let Some(end) = rest.find(quote as char) {
            return Some(&rest[..end]);
        }
    }
    if let Some(close) = matched.find('>') {
        let rest = &matched[close + 1..];
        if let Some(end) = rest.find('<') {
            return Some(&rest[..end]);
        }
    }
    if let Some(colon) = matched.find(':') {
        return Some(matched[colon + 1..].trim());
    }
    if let Some(eq) = matched.find('=') {
        return Some(matched[eq + 1..].trim());
    }
    None
}

/// Bump the top-level project `version = "..."` assignment in a Gradle Kotlin
/// DSL build file (`build.gradle.kts`).
///
/// Gradle build files embed several version-bearing constructs that must NOT be
/// touched:
///   - plugin declarations:  `kotlin("jvm") version "2.3.21"`,
///     `id("org.jlleitschuh.gradle.ktlint") version "1.0.0"`
///   - extension config:      `version.set("1.0.0")` (e.g. the ktlint block)
///   - dependency coordinates: `api("net.java.dev.jna:jna:5.14.0")`
///
/// Only the project version is expressed as a start-of-line `version = "..."`
/// assignment (Gradle Kotlin DSL `Project.version`). The regex anchors to the
/// line start (after optional leading whitespace) and requires the `=`
/// assignment form, so the plugin/extension/coordinate shapes above — which use
/// a space-delimited `version "..."`, a `version.set(...)` call, or no `version`
/// token at all — are left intact.
///
/// Returns the rewritten content when the project version changed, or `None`
/// when the file has no such line or it already matches `new_version`.
pub(super) fn replace_gradle_project_version(content: &str, new_version: &str) -> Option<String> {
    static GRADLE_VERSION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"(?m)^(\s*)version\s*=\s*"[^"]*""#).expect("valid regex"));
    let captures = GRADLE_VERSION_RE.captures(content)?;
    let matched = captures.get(0)?.as_str();
    if matched_version_equals(matched, new_version) {
        return None;
    }
    let indent = captures.get(1).map(|m| m.as_str()).unwrap_or("");
    let replacement = format!(r#"{indent}version = "{new_version}""#);
    let new_content = GRADLE_VERSION_RE.replace(content, replacement.as_str()).into_owned();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Remove the explicit Kotlin Android plugin line when the centralized Android
/// Gradle Plugin pin is 9.0 or newer.
///
/// AGP 9+ provides built-in Kotlin support and rejects re-applying
/// `org.jetbrains.kotlin.android`, so older generated `build.gradle.kts` files
/// need this cleanup during `sync_versions` even though their plugin version pins
/// are otherwise intentionally left untouched.
pub(super) fn remove_stale_kotlin_android_plugin(content: &str) -> Option<String> {
    let agp_major = crate::core::template_versions::maven::ANDROID_GRADLE_PLUGIN
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .unwrap_or(0);
    if agp_major < 9 {
        return None;
    }

    static KOTLIN_ANDROID_PLUGIN_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?m)^\s*kotlin\("android"\)\s+version\s+"[^"]*"\s*\n"#).expect("valid regex")
    });
    let new_content = KOTLIN_ANDROID_PLUGIN_RE.replace_all(content, "").into_owned();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Rewrite the `version = "..."` field of every local/path-source `[[package]]`
/// entry in a committed `Cargo.lock` so it matches the freshly-bumped manifests.
///
/// A binding that ships a committed `Cargo.lock` inside its source tarball (e.g.
/// a Rustler NIF crate packaged into a Hex release) must keep that lockfile in
/// step with the workspace version, otherwise `cargo build` from the published
/// tarball fails with a lock/manifest version mismatch.
///
/// Registry dependencies carry a `source = "registry+..."` (or `git+...`) key
/// and an upstream-pinned version that must never be rewritten. Local crates —
/// the consumer's own workspace members and the NIF crate itself — have NO
/// `source` key and share the workspace version. We bump only those, leaving
/// every registry/git entry untouched.
///
/// The lockfile is line-oriented and `cargo` rewrites it deterministically, so a
/// targeted line rewrite (rather than a full TOML re-serialize) preserves the
/// canonical formatting and avoids reordering. Returns the rewritten content
/// when at least one local entry changed, else `None`.
pub(super) fn sync_cargo_lock_path_versions(content: &str, new_version: &str) -> Option<String> {
    let mut out = String::with_capacity(content.len());
    let mut changed = false;

    let mut block: Vec<&str> = Vec::new();
    let mut in_package_block = false;

    let flush = |block: &mut Vec<&str>, out: &mut String, changed: &mut bool| {
        if block.is_empty() {
            return;
        }
        let is_package = block.first().is_some_and(|l| l.trim() == "[[package]]");
        let has_source = block.iter().any(|l| l.trim_start().starts_with("source = "));
        for line in block.iter() {
            if is_package && !has_source && line.trim_start().starts_with("version = ") {
                let indent_len = line.len() - line.trim_start().len();
                let indent = &line[..indent_len];
                let rewritten = format!(r#"{indent}version = "{new_version}""#);
                if rewritten != *line {
                    *changed = true;
                }
                out.push_str(&rewritten);
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        block.clear();
    };

    for line in content.lines() {
        if line.trim() == "[[package]]" {
            flush(&mut block, &mut out, &mut changed);
            in_package_block = true;
            block.push(line);
        } else if in_package_block {
            block.push(line);
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    flush(&mut block, &mut out, &mut changed);

    if !changed {
        return None;
    }
    if !content.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// Bump the `version-badge` span in generated docs API-reference pages.
///
/// `alef docs` injects the workspace version into the `<span class="version-badge">v…</span>`
/// marker when it regenerates each `api-{lang}.md` heading. A `sync-versions`-only
/// bump (the path consumers take on every release) regenerates READMEs but not
/// the docs tree, so without this the badge stays pinned at the previous version.
/// This rewrites the badge text in-place across all `{docs_reference_dir}/api-*.md`
/// pages so a plain `alef sync-versions` leaves a fully version-consistent tree.
///
/// The match is anchored to the literal `version-badge` span class and the `v`
/// prefix the docs template emits, so unrelated `v…` text in prose is untouched.
/// Returns the list of files whose badge was rewritten.
pub(super) fn sync_docs_version_badges(docs_reference_dir: &std::path::Path, new_version: &str) -> Vec<String> {
    static BADGE_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"(<span class="version-badge">v)[^<]*(</span>)"#).expect("valid regex"));
    let mut updated = Vec::new();
    let pattern = docs_reference_dir.join("api-*.md");
    let Some(pattern_str) = pattern.to_str() else {
        return updated;
    };
    for entry in glob::glob(pattern_str).into_iter().flatten().flatten() {
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let replacement = format!("${{1}}{new_version}${{2}}");
        let new_content = BADGE_RE.replace_all(&content, replacement.as_str()).into_owned();
        if new_content != content {
            if let Err(e) = std::fs::write(&entry, &new_content) {
                debug!("Could not write {}: {e}", entry.display());
            } else {
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }
    updated
}
