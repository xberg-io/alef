//! Docs-only fixtures: hand-authored documentation content that alef validates against the
//! API surface but never executes, generates test code for, or counts as snippet coverage.
//!
//! Some documentation topics do not reduce to "call one function, assert on its result" --
//! the shape every ordinary [`super::Fixture`] and its `docs.presentation` machinery is built
//! around. A topic that walks through a multi-step pipeline, describes how a value is
//! discovered or selected before any call happens, or explains a handoff between two systems
//! has no single call to bind a snippet recipe to. Forcing such a topic into the call-shaped
//! vocabulary either fakes a call that doesn't represent what the prose says, or leaves the
//! topic hand-maintained outside alef entirely -- unverified against the API surface it
//! describes, and free to drift the moment a referenced type, method, or field is renamed.
//!
//! A docs-only fixture is the alternative: an author writes the documentation content
//! directly, declares which API surface items the content is *about*, and alef checks that
//! every declared item still exists. It is never fed to a generator, never rendered into a
//! runnable test or snippet, and never enters [`crate::e2e::snippets::SnippetCoverageLedger`].
//!
//! ## Why this can't be an accidental spelling of [`super::Fixture`]
//!
//! A docs-only fixture is a distinct Rust type, not an optional mode of `Fixture`. Two
//! properties fall out of that:
//!
//! - **A fixture cannot become docs-only by accident.** [`DocsOnlyFixture`] requires an
//!   explicit `"kind": "docs_only"` field with no default, and rejects any unrecognized key
//!   (`#[serde(deny_unknown_fields)]`) -- so a file cannot carry both `docs_only` framing and
//!   runtime fields like `call`, `assertions`, or `http`; those fields don't exist on this
//!   type at all. [`crate::e2e::fixture::load_fixtures`] (the loader every runtime codegen
//!   path calls) refuses to parse a `"kind": "docs_only"` file as a [`super::Fixture`] -- it
//!   skips the file outright (see [`is_docs_only_marker`] and its call site in
//!   `load_fixtures_recursive`) rather than silently accepting it as a trivial always-passing
//!   smoke test.
//! - **A docs-only fixture cannot be silently counted as runtime-covered.** It never becomes a
//!   [`super::Fixture`] value (there is no conversion between the two types), so it never
//!   reaches [`crate::e2e::codegen::all_generators`], [`crate::e2e::snippets::generate_snippet_report`],
//!   or any [`crate::e2e::snippets::SnippetCoverageLedger`] field. Its own output tree
//!   ([`DOCS_ONLY_OUTPUT_SLUG`]) is disjoint from every language snippet slug, so its files
//!   cannot collide with a generated snippet path either.
//!
//! ## What "validated but never executed" means here
//!
//! [`validate_api_references`] resolves every [`ApiReference`] a fixture declares against the
//! crate's extracted IR (`type_defs`, `enums`, `errors`, `functions`) -- the same slices every
//! other e2e validation pass in this crate already receives. A reference naming a type,
//! function, method, field, or enum/error variant that does not exist fails the run with the
//! offending fixture id and path named. Nothing about the declared `content` is type-checked,
//! compiled, or run: this is a structural-accuracy check ("does this documentation still talk
//! about a thing that exists"), not an execution check.

use crate::core::ir::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// The literal `kind` value that marks a fixture file as docs-only.
///
/// The only string [`DocsOnlyKind`] accepts. Exposed so callers building an error message (or
/// a schema) can name it rather than repeating the literal.
pub const DOCS_ONLY_FIXTURE_KIND: &str = "docs_only";

/// Output directory slug docs-only fixtures render under, parallel to the per-language slugs
/// [`crate::e2e::snippets`] uses (`python`, `typescript`, `kotlin-android`, ...).
///
/// Not a real language spelling, so it can never collide with one -- see the module doc's
/// "cannot be silently counted as runtime-covered" guarantee.
pub const DOCS_ONLY_OUTPUT_SLUG: &str = "docs-only";

/// The required, single-value marker that makes `"kind": "docs_only"` mandatory rather than a
/// default. Omitting the field, or spelling it any other way, is a parse error -- there is no
/// way to reach [`DocsOnlyFixture`] without writing this literally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocsOnlyKind {
    DocsOnly,
}

/// A single API surface item a docs-only fixture's content is about.
///
/// `Method`, `Field`, and `Variant` paths are `"Owner.member"` -- the owning type/enum/error
/// name, a literal `.`, then the member name. `Type` and `Function` paths name a single
/// top-level item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApiReference {
    /// A struct, enum, or error type by name (`type_defs`, `enums`, or `errors`).
    Type { path: String },
    /// A top-level free function by name (`functions`).
    Function { path: String },
    /// `"Owner.method"` -- an instance or associated method on a type, enum, or error.
    Method { path: String },
    /// `"Owner.field"` -- a struct field. Enum/error variant fields are not resolved by this
    /// kind; see [`ApiReference::Variant`] for the variant itself.
    Field { path: String },
    /// `"Owner.Variant"` -- an enum or error variant.
    Variant { path: String },
}

impl ApiReference {
    fn describe(&self) -> String {
        match self {
            Self::Type { path } => format!("type `{path}`"),
            Self::Function { path } => format!("function `{path}`"),
            Self::Method { path } => format!("method `{path}`"),
            Self::Field { path } => format!("field `{path}`"),
            Self::Variant { path } => format!("variant `{path}`"),
        }
    }

    fn path(&self) -> &str {
        match self {
            Self::Type { path }
            | Self::Function { path }
            | Self::Method { path }
            | Self::Field { path }
            | Self::Variant { path } => path,
        }
    }

    /// Split an `"Owner.member"` path into its two halves, failing if either half is empty
    /// (including a bare `"Owner."` or a path with no `.` at all).
    fn owner_and_member(&self) -> Result<(&str, &str)> {
        let path = self.path();
        let (owner, member) = path
            .split_once('.')
            .ok_or_else(|| anyhow::anyhow!("{} must be written as `Owner.member`", self.describe()))?;
        if owner.is_empty() || member.is_empty() || member.contains('.') {
            bail!("{} must be written as exactly one `Owner.member`", self.describe());
        }
        Ok((owner, member))
    }

    /// Resolve this reference against the extracted API surface, failing with a message that
    /// names both the reference and the fixture it came from.
    fn resolve(&self, surface: &ApiSurfaceView<'_>) -> Result<()> {
        match self {
            Self::Type { path } => {
                if surface.has_type(path) {
                    Ok(())
                } else {
                    bail!("no type named `{path}` exists in the API surface")
                }
            }
            Self::Function { path } => {
                if surface.functions.iter().any(|function| &function.name == path) {
                    Ok(())
                } else {
                    bail!("no function named `{path}` exists in the API surface")
                }
            }
            Self::Method { .. } => {
                let (owner, member) = self.owner_and_member()?;
                if surface.has_method(owner, member) {
                    Ok(())
                } else {
                    bail!("no method `{member}` exists on `{owner}` in the API surface")
                }
            }
            Self::Field { .. } => {
                let (owner, member) = self.owner_and_member()?;
                if surface.has_field(owner, member) {
                    Ok(())
                } else {
                    bail!("no field `{member}` exists on `{owner}` in the API surface")
                }
            }
            Self::Variant { .. } => {
                let (owner, member) = self.owner_and_member()?;
                if surface.has_variant(owner, member) {
                    Ok(())
                } else {
                    bail!("no variant `{member}` exists on `{owner}` in the API surface")
                }
            }
        }
    }
}

/// A read-only view over the four IR slices a docs-only reference can resolve against, built
/// once per validation call rather than re-scanning the slices for every reference.
struct ApiSurfaceView<'a> {
    type_defs: &'a [TypeDef],
    enums: &'a [EnumDef],
    errors: &'a [ErrorDef],
    functions: &'a [FunctionDef],
}

impl<'a> ApiSurfaceView<'a> {
    fn has_type(&self, name: &str) -> bool {
        self.type_defs.iter().any(|type_def| type_def.name == name)
            || self.enums.iter().any(|enum_def| enum_def.name == name)
            || self.errors.iter().any(|error_def| error_def.name == name)
    }

    fn has_method(&self, owner: &str, method: &str) -> bool {
        self.type_defs
            .iter()
            .find(|type_def| type_def.name == owner)
            .map(|type_def| type_def.methods.iter().any(|m| m.name == method))
            .or_else(|| {
                self.enums
                    .iter()
                    .find(|enum_def| enum_def.name == owner)
                    .map(|enum_def| enum_def.methods.iter().any(|m| m.name == method))
            })
            .or_else(|| {
                self.errors
                    .iter()
                    .find(|error_def| error_def.name == owner)
                    .map(|error_def| error_def.methods.iter().any(|m| m.name == method))
            })
            .unwrap_or(false)
    }

    fn has_field(&self, owner: &str, field: &str) -> bool {
        self.type_defs
            .iter()
            .find(|type_def| type_def.name == owner)
            .is_some_and(|type_def| type_def.fields.iter().any(|f| f.name == field))
    }

    fn has_variant(&self, owner: &str, variant: &str) -> bool {
        self.enums
            .iter()
            .find(|enum_def| enum_def.name == owner)
            .map(|enum_def| enum_def.variants.iter().any(|v| v.name == variant))
            .or_else(|| {
                self.errors
                    .iter()
                    .find(|error_def| error_def.name == owner)
                    .map(|error_def| error_def.variants.iter().any(|v| v.name == variant))
            })
            .unwrap_or(false)
    }
}

/// A docs-only fixture: hand-authored documentation content plus the API surface items it is
/// about. See the module doc for what this is and why it exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocsOnlyFixture {
    /// Must be `"docs_only"`. See [`DocsOnlyKind`].
    pub kind: DocsOnlyKind,
    /// Unique identifier (used as the output file stem when `stem` is absent).
    pub id: String,
    /// Optional category, informational only -- docs-only fixtures have no generated test
    /// grouping to key off of.
    #[serde(default)]
    pub category: Option<String>,
    /// Human-readable description, informational only.
    #[serde(default)]
    pub description: String,
    /// Output subdirectory (mirrors [`super::FixtureDocs::topic`]).
    pub topic: String,
    /// Output file stem, defaulting to `id`.
    #[serde(default)]
    pub stem: Option<String>,
    /// Optional front-matter title.
    #[serde(default)]
    pub title: Option<String>,
    /// API surface items this fixture's content is about. Every entry must resolve against
    /// the extracted IR or [`validate_api_references`] fails the run.
    #[serde(default)]
    pub references: Vec<ApiReference>,
    /// The hand-authored documentation content (Markdown, including any code the author
    /// wrote directly). Never templated, type-checked, or executed by alef.
    pub content: String,
    /// Source file path, populated during loading.
    #[serde(skip)]
    pub source: String,
}

/// True when `value`'s top-level `"kind"` field is the docs-only marker.
///
/// Used by `crate::e2e::fixture::load_fixtures_recursive` to skip a docs-only file before it
/// is ever handed to [`super::Fixture`]'s (permissive, `#[serde(default)]`-everywhere)
/// deserializer -- which would otherwise happily parse it as a trivial fixture with no
/// assertions, silently double-booking the same file as both docs-only content and a
/// runtime "just call it" smoke test.
pub fn is_docs_only_marker(value: &serde_json::Value) -> bool {
    value.get("kind").and_then(serde_json::Value::as_str) == Some(DOCS_ONLY_FIXTURE_KIND)
}

fn validate_path_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || Path::new(value).components().count() != 1 || matches!(value, "." | "..") {
        bail!("unsafe {label} `{value}`");
    }
    Ok(())
}

fn validate_docs_only_fixture_shape(fixture: &DocsOnlyFixture) -> Result<()> {
    if fixture.id.trim().is_empty() {
        bail!("docs-only fixture has an empty id");
    }
    if fixture.content.trim().is_empty() {
        bail!("docs-only fixture `{}` has empty content", fixture.id);
    }
    validate_path_component(&fixture.topic, "docs-only topic").with_context(|| fixture.id.clone())?;
    let stem = fixture.stem.as_deref().unwrap_or(&fixture.id);
    validate_path_component(stem, "docs-only stem").with_context(|| fixture.id.clone())?;
    for reference in &fixture.references {
        if !matches!(reference, ApiReference::Type { .. } | ApiReference::Function { .. }) {
            reference.owner_and_member().with_context(|| {
                format!(
                    "docs-only fixture `{}` has an invalid reference {}",
                    fixture.id,
                    reference.describe()
                )
            })?;
        } else if reference.path().trim().is_empty() {
            bail!(
                "docs-only fixture `{}` has an empty path for {}",
                fixture.id,
                reference.describe()
            );
        }
    }
    Ok(())
}

/// Load every docs-only fixture under `dir`, recursively.
///
/// Mirrors [`super::load_fixtures`]'s directory walk (sorted paths, `schema.json` and
/// `_`-prefixed files skipped) but collects only files whose top-level JSON carries
/// `"kind": "docs_only"` -- every other file is left untouched for `load_fixtures` to load as
/// a runtime [`super::Fixture`]. A docs-only fixture must be a single top-level JSON object;
/// the array-of-fixtures shape `load_fixtures` accepts is not supported here, since a docs-only
/// fixture has no natural grouping with sibling fixtures the way runtime call variants do.
pub fn load_docs_only_fixtures(dir: &Path) -> Result<Vec<DocsOnlyFixture>> {
    let mut fixtures = Vec::new();
    load_docs_only_recursive(dir, dir, &mut fixtures)?;

    let mut seen: HashMap<String, String> = HashMap::new();
    for fixture in &fixtures {
        if let Some(previous_source) = seen.get(&fixture.id) {
            bail!(
                "duplicate docs-only fixture ID '{}': found in '{}' and '{}'",
                fixture.id,
                previous_source,
                fixture.source
            );
        }
        seen.insert(fixture.id.clone(), fixture.source.clone());
    }

    fixtures.sort_by(|a, b| a.topic.cmp(&b.topic).then_with(|| a.id.cmp(&b.id)));
    Ok(fixtures)
}

fn load_docs_only_recursive(base: &Path, dir: &Path, fixtures: &mut Vec<DocsOnlyFixture>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_docs_only_recursive(base, &path, fixtures)?;
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let filename = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if filename == "schema.json" || filename.starts_with('_') {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read docs-only fixture: {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse docs-only fixture candidate: {}", path.display()))?;
        if !is_docs_only_marker(&value) {
            continue;
        }
        let mut fixture: DocsOnlyFixture = serde_json::from_value(value)
            .with_context(|| format!("failed to parse docs-only fixture: {}", path.display()))?;
        fixture.source = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();
        validate_docs_only_fixture_shape(&fixture)
            .with_context(|| format!("invalid docs-only fixture: {}", path.display()))?;
        fixtures.push(fixture);
    }
    Ok(())
}

/// Resolve every [`ApiReference`] a docs-only fixture declares against the extracted API
/// surface, failing with the fixture id, source path, and offending reference named as soon as
/// the first one does not resolve.
pub fn validate_api_references(
    fixture: &DocsOnlyFixture,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    errors: &[ErrorDef],
    functions: &[FunctionDef],
) -> Result<()> {
    let surface = ApiSurfaceView {
        type_defs,
        enums,
        errors,
        functions,
    };
    for reference in &fixture.references {
        reference.resolve(&surface).with_context(|| {
            format!(
                "docs-only fixture `{}` ({}) references {} that does not exist",
                fixture.id,
                fixture.source,
                reference.describe()
            )
        })?;
    }
    Ok(())
}

fn validate_relative_output(output: &str) -> Result<()> {
    let path = Path::new(output);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("docs-only output root must be a safe relative path: {output}");
    }
    Ok(())
}

/// The project-root-relative output path a docs-only fixture renders to:
/// `<output>/docs-only/<topic>/<stem or id>.md`.
pub fn docs_only_output_path(output: &str, fixture: &DocsOnlyFixture) -> Result<PathBuf> {
    validate_relative_output(output)?;
    let stem = fixture.stem.as_deref().unwrap_or(&fixture.id);
    Ok(Path::new(output)
        .join(DOCS_ONLY_OUTPUT_SLUG)
        .join(&fixture.topic)
        .join(format!("{stem}.md")))
}

/// The CLI invocation that produces docs-only fixture files, embedded in the same provenance
/// header [`crate::e2e::snippets`] stamps on generated snippets.
const DOCS_ONLY_REGENERATE_COMMAND: &str = "alef e2e generate";

/// Escape `value` as a double-quoted YAML scalar for front matter, so a title containing a
/// colon or quote cannot break the surrounding front-matter block.
fn yaml_quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn render_docs_only_markdown(fixture: &DocsOnlyFixture) -> String {
    let rendered = crate::e2e::template_env::render(
        "snippets/docs_only_file.md.jinja",
        crate::alef_context! {
            id => format!("docs_only_{}", fixture.id),
            topic => &fixture.topic,
            title_yaml => fixture.title.as_deref().map(yaml_quoted),
            content => fixture.content.trim_end(),
        },
    );
    crate::docs::with_html_header(rendered, DOCS_ONLY_REGENERATE_COMMAND)
}

/// Render one validated docs-only fixture into its output file.
///
/// Callers must run [`validate_api_references`] first -- this function renders unconditionally
/// and does not itself re-check references.
pub fn render_docs_only_fixture(
    fixture: &DocsOnlyFixture,
    output: &str,
) -> Result<crate::core::backend::GeneratedFile> {
    let path = docs_only_output_path(output, fixture)?;
    Ok(crate::core::backend::GeneratedFile {
        path,
        content: render_docs_only_markdown(fixture),
        generated_header: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_def(name: &str, fields: &[&str], methods: &[&str]) -> TypeDef {
        let mut type_def: TypeDef = serde_json::from_value(serde_json::json!({
            "name": name,
            "rust_path": format!("crate::{name}"),
            "fields": [],
            "methods": [],
            "is_opaque": false,
            "is_clone": false,
            "doc": "",
        }))
        .unwrap();
        type_def.fields = fields
            .iter()
            .map(|field| {
                serde_json::from_value(serde_json::json!({
                    "name": field,
                    "ty": "String",
                    "optional": false,
                    "default": null,
                    "doc": "",
                }))
                .unwrap()
            })
            .collect();
        type_def.methods = methods
            .iter()
            .map(|method| {
                serde_json::from_value(serde_json::json!({
                    "name": method,
                    "params": [],
                    "return_type": "String",
                    "is_async": false,
                    "is_static": false,
                    "error_type": null,
                    "doc": "",
                    "receiver": null,
                }))
                .unwrap()
            })
            .collect();
        type_def
    }

    fn function_def(name: &str) -> FunctionDef {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "rust_path": format!("crate::{name}"),
            "params": [],
            "return_type": "String",
            "is_async": false,
            "error_type": null,
            "doc": "",
        }))
        .unwrap()
    }

    fn docs_only_json(references: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "kind": "docs_only",
            "id": "config_discovery",
            "topic": "guides",
            "content": "Configuration is discovered by walking up from the working directory.",
            "references": references,
        })
    }

    #[test]
    fn kind_field_is_required_and_must_be_the_exact_literal() {
        let missing = serde_json::json!({
            "id": "config_discovery",
            "topic": "guides",
            "content": "text",
        });
        assert!(
            serde_json::from_value::<DocsOnlyFixture>(missing).is_err(),
            "omitting `kind` must fail to parse as DocsOnlyFixture"
        );

        let wrong_value = serde_json::json!({
            "kind": "docs-only",
            "id": "config_discovery",
            "topic": "guides",
            "content": "text",
        });
        assert!(
            serde_json::from_value::<DocsOnlyFixture>(wrong_value).is_err(),
            "a near-miss spelling must not be accepted"
        );
    }

    /// The structural half of "impossible to mark docs-only by accident": a file that mixes
    /// the docs-only marker with a runtime field is rejected at parse time, not silently
    /// stripped of one shape or the other.
    #[test]
    fn deny_unknown_fields_rejects_a_runtime_field_on_a_docs_only_fixture() {
        let mixed = serde_json::json!({
            "kind": "docs_only",
            "id": "config_discovery",
            "topic": "guides",
            "content": "text",
            "call": "discover_config",
        });
        let error = serde_json::from_value::<DocsOnlyFixture>(mixed)
            .expect_err("a docs-only fixture carrying a runtime `call` field must be rejected");
        assert!(error.to_string().contains("call"), "{error}");
    }

    /// The other structural half: `crate::e2e::fixture::load_fixtures` -- the loader every
    /// runtime codegen path calls -- must never turn a docs-only file into a `Fixture`.
    #[test]
    fn load_fixtures_skips_a_docs_only_marked_file() {
        let dir = tempfile_dir();
        std::fs::write(
            dir.join("docs_only_example.json"),
            docs_only_json(serde_json::json!([])).to_string(),
        )
        .unwrap();

        let runtime_fixtures = super::super::load_fixtures(&dir).expect("runtime loader must not error");
        assert!(
            runtime_fixtures.is_empty(),
            "a docs-only file must never be parsed as a runtime Fixture: got {runtime_fixtures:?}"
        );

        let docs_only_fixtures = load_docs_only_fixtures(&dir).expect("docs-only loader must find the file");
        assert_eq!(docs_only_fixtures.len(), 1);
        assert_eq!(docs_only_fixtures[0].id, "config_discovery");
    }

    #[test]
    fn a_reference_to_a_real_type_and_field_validates_clean() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([
            {"kind": "type", "path": "ConfigSource"},
            {"kind": "field", "path": "ConfigSource.priority"},
            {"kind": "function", "path": "discover_config"},
        ])))
        .unwrap();
        let type_defs = vec![type_def("ConfigSource", &["priority"], &[])];
        let functions = vec![function_def("discover_config")];

        validate_api_references(&fixture, &type_defs, &[], &[], &functions)
            .expect("every reference names a real API surface item");
    }

    /// The point of the whole feature: a reference to a field that does not exist fails
    /// validation. Without this, docs-only fixtures would be indistinguishable from a feature
    /// that just skips checking them.
    #[test]
    fn a_reference_to_a_field_that_does_not_exist_fails_validation() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([
            {"kind": "field", "path": "ConfigSource.does_not_exist"},
        ])))
        .unwrap();
        let type_defs = vec![type_def("ConfigSource", &["priority"], &[])];

        let error = validate_api_references(&fixture, &type_defs, &[], &[], &[])
            .expect_err("a field that does not exist must fail validation");
        assert!(error.to_string().contains("does_not_exist"), "{error}");
        assert!(error.to_string().contains("config_discovery"), "{error}");
    }

    #[test]
    fn a_reference_to_a_method_that_does_not_exist_fails_validation() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([
            {"kind": "method", "path": "ConfigSource.reload"},
        ])))
        .unwrap();
        let type_defs = vec![type_def("ConfigSource", &[], &["load"])];

        let error = validate_api_references(&fixture, &type_defs, &[], &[], &[])
            .expect_err("a method that does not exist must fail validation");
        assert!(error.to_string().contains("reload"), "{error}");
    }

    #[test]
    fn a_reference_to_a_type_that_does_not_exist_fails_validation() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([
            {"kind": "type", "path": "NoSuchType"},
        ])))
        .unwrap();

        let error =
            validate_api_references(&fixture, &[], &[], &[], &[]).expect_err("an unknown type must fail validation");
        assert!(error.to_string().contains("NoSuchType"), "{error}");
    }

    #[test]
    fn method_and_field_paths_must_be_owner_dot_member() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([
            {"kind": "field", "path": "no_dot_here"},
        ])))
        .unwrap();
        let error = validate_docs_only_fixture_shape(&fixture).expect_err("a bare path must be rejected at load time");
        let full = format!("{error:#}");
        assert!(full.contains("Owner.member"), "{full}");
    }

    #[test]
    fn empty_content_is_rejected_at_load_time() {
        let mut fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([]))).unwrap();
        fixture.content = "   ".to_string();
        let error = validate_docs_only_fixture_shape(&fixture).expect_err("empty content must be rejected");
        assert!(error.to_string().contains("empty content"), "{error}");
    }

    #[test]
    fn output_path_uses_the_dedicated_slug_and_stem_default() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([]))).unwrap();
        let path = docs_only_output_path("docs/snippets-generated", &fixture).unwrap();
        assert_eq!(
            path,
            Path::new("docs/snippets-generated/docs-only/guides/config_discovery.md")
        );
    }

    #[test]
    fn stem_override_wins_over_id() {
        let mut fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([]))).unwrap();
        fixture.stem = Some("discovery-order".to_string());
        let path = docs_only_output_path("out", &fixture).unwrap();
        assert_eq!(path, Path::new("out/docs-only/guides/discovery-order.md"));
    }

    #[test]
    fn rendered_output_carries_the_alef_provenance_marker() {
        let fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([]))).unwrap();
        let file = render_docs_only_fixture(&fixture, "docs/snippets-generated").unwrap();
        assert!(
            crate::core::hash::content_has_alef_marker(&file.content),
            "rendered docs-only output must be recognized as alef-owned: {}",
            file.content
        );
        assert!(file.content.contains("kind: docs_only"));
        assert!(file.content.contains("Configuration is discovered"));
    }

    #[test]
    fn title_is_yaml_escaped_in_front_matter() {
        let mut fixture: DocsOnlyFixture = serde_json::from_value(docs_only_json(serde_json::json!([]))).unwrap();
        fixture.title = Some("Config: the \"discovery\" order".to_string());
        let file = render_docs_only_fixture(&fixture, "out").unwrap();
        assert!(
            file.content.contains("title: \"Config: the \\\"discovery\\\" order\""),
            "{}",
            file.content
        );
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alef-docs-only-test-{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}-{:?}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
            std::thread::current().id()
        )
    }
}
