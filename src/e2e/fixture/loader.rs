//! Directory walk and per-file parsing for runtime [`super::Fixture`] loading.
//!
//! Split out of `fixture.rs` (which owns the `Fixture` type itself, its validation helpers,
//! and grouping) so the loading concern -- reading a directory tree, telling a docs-only file
//! apart from a runtime one, parsing JSON, and normalizing/expanding it -- has its own file
//! rather than growing the already over-cap parent. See CLAUDE.md's `file-modularization` rule.

use super::Fixture;
use super::HttpFixture;
use super::docs_only;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;

/// Load all fixtures from a directory recursively.
pub fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>> {
    let mut fixtures = Vec::new();
    load_fixtures_recursive(dir, dir, &mut fixtures)?;

    // Validate: check for duplicate IDs
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in &fixtures {
        if let Some(prev_source) = seen.get(&f.id) {
            bail!(
                "duplicate fixture ID '{}': found in '{}' and '{}'",
                f.id,
                prev_source,
                f.source
            );
        }
        seen.insert(f.id.clone(), f.source.clone());
    }

    // Sort by (category, id) for deterministic output
    fixtures.sort_by(|a, b| {
        let cat_cmp = a.resolved_category().cmp(&b.resolved_category());
        cat_cmp.then_with(|| a.id.cmp(&b.id))
    });

    Ok(fixtures)
}

fn load_fixtures_recursive(base: &Path, dir: &Path, fixtures: &mut Vec<Fixture>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_fixtures_recursive(base, &path, fixtures)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read fixture: {}", path.display()))?;
            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            if skip_docs_only_fixture(&content, &path)? {
                continue;
            }

            // Try parsing as array first, then as single fixture. Normalize at the
            // raw JSON level so fixture-level helper fields that are not stored on
            // `Fixture` can still influence generated argument input.
            let parsed: Vec<Fixture> = if content.trim_start().starts_with('[') {
                let values: Vec<serde_json::Value> = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?;
                values
                    .into_iter()
                    .map(deserialize_fixture_value)
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?
            } else {
                let value: serde_json::Value = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                let single = deserialize_fixture_value(value)
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                vec![single]
            };

            for mut fixture in parsed {
                fixture.source = relative.clone();
                validate_docs_file_inputs(&fixture)
                    .with_context(|| format!("invalid docs file input in fixture: {}", path.display()))?;
                validate_docs_metadata(&fixture)
                    .with_context(|| format!("invalid docs metadata in fixture: {}", path.display()))?;
                // Expand template expressions (e.g. `{{ repeat 'x' 10000 times }}`)
                // in all JSON string values so generators emit the expanded values.
                expand_json_templates(&mut fixture.input);
                if let Some(ref mut http) = fixture.http {
                    expand_http_fixture_templates(http);
                }
                fixtures.push(fixture);
            }
        }
    }
    Ok(())
}

/// True when `path` is owned by `docs_only::load_docs_only_fixtures` and must not be parsed as
/// a runtime [`Fixture`] here.
///
/// A docs-only fixture (`"kind": "docs_only"`) is owned by a separate walk of this same tree.
/// Skipping it here -- rather than letting it fall through to `Fixture`'s permissive,
/// all-`#[serde(default)]` deserializer -- is what keeps a docs-only file from silently
/// becoming a trivial, always-passing runtime smoke test. See `docs_only`'s module doc. ~keep
fn skip_docs_only_fixture(content: &str, path: &Path) -> Result<bool> {
    let Ok(peek) = serde_json::from_str::<serde_json::Value>(content) else {
        return Ok(false);
    };
    if docs_only::is_docs_only_marker(&peek) {
        return Ok(true);
    }
    if let Some(array) = peek.as_array()
        && array.iter().any(docs_only::is_docs_only_marker)
    {
        bail!(
            "docs-only fixtures must be a single top-level JSON object, not inside a fixture array: {}",
            path.display()
        );
    }
    Ok(false)
}

fn validate_docs_metadata(fixture: &Fixture) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    let fixture_expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if docs.error == Some(true) && !fixture_expects_error {
        bail!("docs.error requires an error assertion");
    }
    if docs.error == Some(false) && fixture_expects_error {
        bail!("docs.error cannot be false when the fixture expects an error");
    }
    if docs.shows.iter().any(|path| path.trim().is_empty()) {
        bail!("docs.shows entries must be non-empty result paths");
    }
    Ok(())
}

fn validate_docs_file_inputs(fixture: &Fixture) -> Result<()> {
    let Some(presentation) = fixture.docs.as_ref().and_then(|docs| docs.presentation.as_ref()) else {
        return Ok(());
    };
    let input = presentation.input.as_ref().unwrap_or(&fixture.input);
    for file in &presentation.files {
        let path = Path::new(&file.path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            bail!("docs file path must be relative and traversal-free: {}", file.path);
        }
        if !file.field.starts_with('/') || input.pointer(&file.field).is_none() {
            bail!("docs file field must be an existing JSON pointer: {}", file.field);
        }
    }
    Ok(())
}

fn normalize_fixture_value(mut value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };

    if let Some(config) = object.get("config").cloned() {
        let input = object
            .entry("input")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(input_object) = input.as_object_mut() {
            input_object.entry("config".to_string()).or_insert(config);
        }
    }

    value
}

fn deserialize_fixture_value(mut value: serde_json::Value) -> Result<Fixture> {
    let carrier_timeout_ms = timeout_from_carrier_tag(&value)?;
    let test_timeout_ms = value
        .as_object_mut()
        .and_then(|object| object.remove("test_timeout_ms"))
        .map(|timeout| parse_test_timeout_ms(&timeout))
        .transpose()?;
    if test_timeout_ms.is_some() && carrier_timeout_ms.is_some() {
        bail!("test_timeout_ms conflicts with the reserved timeout carrier tag");
    }
    let mut fixture: Fixture = serde_json::from_value(normalize_fixture_value(value))?;
    if let Some(test_timeout_ms) = test_timeout_ms {
        fixture
            .tags
            .push(format!("{}{test_timeout_ms}", Fixture::TEST_TIMEOUT_TAG_PREFIX));
    }
    Ok(fixture)
}

fn parse_test_timeout_ms(value: &serde_json::Value) -> Result<u64> {
    let timeout = value.as_u64().context("test_timeout_ms must be a positive integer")?;
    if !(1..=Fixture::MAX_TEST_TIMEOUT_MS).contains(&timeout) {
        bail!("test_timeout_ms must be between 1 and {}", Fixture::MAX_TEST_TIMEOUT_MS);
    }
    Ok(timeout)
}

fn timeout_from_carrier_tag(value: &serde_json::Value) -> Result<Option<u64>> {
    let Some(tags) = value.get("tags").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };
    let mut timeout = None;
    for tag in tags.iter().filter_map(serde_json::Value::as_str) {
        if !tag.starts_with(Fixture::TEST_TIMEOUT_TAG_NAMESPACE) {
            continue;
        }
        let raw_timeout = tag
            .strip_prefix(Fixture::TEST_TIMEOUT_TAG_PREFIX)
            .context("malformed reserved timeout carrier tag")?;
        let parsed_timeout = raw_timeout
            .parse::<u64>()
            .context("malformed reserved timeout carrier tag")?;
        parse_test_timeout_ms(&serde_json::Value::from(parsed_timeout))?;
        if timeout.replace(parsed_timeout).is_some() {
            bail!("fixture has more than one reserved timeout carrier tag");
        }
    }
    Ok(timeout)
}

/// Recursively expand fixture template expressions in all string values of a JSON tree.
fn expand_json_templates(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            let expanded = crate::e2e::escape::expand_fixture_templates(s);
            if expanded != *s {
                *s = expanded;
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                expand_json_templates(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                expand_json_templates(v);
            }
        }
        _ => {}
    }
}

/// Expand fixture template expressions in an HTTP fixture's request headers and body.
fn expand_http_fixture_templates(http: &mut HttpFixture) {
    for v in http.request.headers.values_mut() {
        *v = crate::e2e::escape::expand_fixture_templates(v);
    }
    if let Some(ref mut body) = http.request.body {
        expand_json_templates(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_fixture_value_copies_top_level_config_into_input() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {"kind": "uri", "uri": "doc.txt"},
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("markdown"))
        );
    }

    #[test]
    fn normalize_fixture_value_preserves_explicit_input_config() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {
                "kind": "uri",
                "uri": "doc.txt",
                "config": {"output_format": "html"}
            },
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("html"))
        );
    }

    #[test]
    fn fixture_test_timeout_round_trips_as_generator_metadata() {
        let value = serde_json::json!({
            "id": "bounded_call",
            "description": "Bounded call",
            "test_timeout_ms": 60_000
        });

        let fixture = deserialize_fixture_value(value).unwrap();

        assert_eq!(fixture.test_timeout_ms(), Some(60_000));
        let serialized = serde_json::to_value(&fixture).unwrap();
        assert!(
            serialized.get("test_timeout_ms").is_none() && serialized.pointer("/env/test_timeout_ms").is_none(),
            "the loader-only carrier must not become a fixture field"
        );
        assert_eq!(
            serialized.pointer("/tags/0"),
            Some(&serde_json::json!("alef:test_timeout_ms=60000"))
        );
        let round_tripped = deserialize_fixture_value(serialized).unwrap();
        assert_eq!(round_tripped.test_timeout_ms(), Some(60_000));
    }

    #[test]
    fn fixture_test_timeout_loads_from_fixture_arrays() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixtures.json"),
            r#"[
                {"id": "first", "description": "first", "test_timeout_ms": 60000},
                {"id": "second", "description": "second"}
            ]"#,
        )
        .unwrap();

        let fixtures = load_fixtures(directory.path()).unwrap();

        assert_eq!(fixtures[0].test_timeout_ms(), Some(60_000));
        assert_eq!(fixtures[1].test_timeout_ms(), None);
    }

    #[test]
    fn fixture_test_timeout_rejects_malformed_or_duplicate_carrier_tags() {
        for tags in [
            serde_json::json!(["alef:test_timeout_ms="]),
            serde_json::json!(["alef:test_timeout_ms=0"]),
            serde_json::json!(["alef:test_timeout_ms=1.5"]),
            serde_json::json!(["alef:test_timeout_ms=2147483648"]),
            serde_json::json!(["alef:test_timeout_ms:60000"]),
            serde_json::json!(["alef:test_timeout_ms=60000", "alef:test_timeout_ms=60000"]),
            serde_json::json!(["alef:test_timeout_ms=60000", "alef:test_timeout_ms=90000"]),
        ] {
            let value = serde_json::json!({
                "id": "reserved_tag",
                "description": "reserved tag",
                "tags": tags
            });

            assert!(deserialize_fixture_value(value).is_err());
        }
    }

    #[test]
    fn fixture_test_timeout_rejects_conflicting_sources() {
        let value = serde_json::json!({
            "id": "conflicting_timeout",
            "description": "conflicting timeout",
            "test_timeout_ms": 60_000,
            "tags": ["alef:test_timeout_ms=60000"]
        });

        let error = deserialize_fixture_value(value).expect_err("conflicting timeout sources must fail");
        assert!(error.to_string().contains("conflicts"));
    }

    #[test]
    fn fixture_test_timeout_enforces_bounds_without_schema_validation() {
        for timeout in [
            serde_json::json!(0),
            serde_json::json!(2_147_483_648_u64),
            serde_json::json!("60000"),
        ] {
            let value = serde_json::json!({
                "id": "invalid_timeout",
                "description": "invalid timeout",
                "test_timeout_ms": timeout
            });

            assert!(deserialize_fixture_value(value).is_err());
        }
    }

    #[test]
    fn docs_file_inputs_require_safe_existing_pointers() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "file_input",
            "description": "Reads a local document",
            "input": {"content": [1, 2, 3]},
            "assertions": [],
            "docs": {
                "topic": "guides",
                "presentation": {"files": [{"field": "/content", "path": "../secret.pdf"}]}
            }
        }))
        .expect("fixture");

        let error = validate_docs_file_inputs(&fixture).expect_err("traversal must be rejected");
        assert!(error.to_string().contains("traversal-free"), "{error}");

        let mut missing = fixture;
        let file = &mut missing
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .files[0];
        file.path = "document.pdf".into();
        file.field = "/missing".into();
        let error = validate_docs_file_inputs(&missing).expect_err("missing pointers must be rejected");
        assert!(error.to_string().contains("existing JSON pointer"), "{error}");
    }

    /// A docs-only file must never fall through to `Fixture`'s permissive deserializer. See
    /// `docs_only`'s own tests for the loader-isolation guarantee from the other side.
    #[test]
    fn skip_docs_only_fixture_is_true_for_a_docs_only_marked_object() {
        let content = serde_json::json!({
            "kind": "docs_only",
            "id": "config_discovery",
            "topic": "guides",
            "content": "text",
        })
        .to_string();
        assert!(skip_docs_only_fixture(&content, Path::new("fixture.json")).unwrap());
    }

    #[test]
    fn skip_docs_only_fixture_is_false_for_an_ordinary_fixture() {
        let content = serde_json::json!({
            "id": "ordinary",
            "description": "not docs-only",
        })
        .to_string();
        assert!(!skip_docs_only_fixture(&content, Path::new("fixture.json")).unwrap());
    }

    #[test]
    fn skip_docs_only_fixture_rejects_a_docs_only_entry_inside_an_array() {
        let content = serde_json::json!([
            {"kind": "docs_only", "id": "a", "topic": "guides", "content": "text"},
        ])
        .to_string();
        let error =
            skip_docs_only_fixture(&content, Path::new("fixture.json")).expect_err("array-wrapped must be rejected");
        assert!(error.to_string().contains("single top-level JSON object"), "{error}");
    }
}
