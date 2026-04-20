//! Fixture loading, validation, and grouping for e2e test generation.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Mock HTTP response for testing HTTP clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    /// HTTP status code.
    pub status: u16,
    /// JSON response body (for non-streaming responses).
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// SSE stream chunks (for streaming responses).
    /// Each chunk is a JSON object sent as `data: <chunk>\n\n`.
    #[serde(default)]
    pub stream_chunks: Option<Vec<serde_json::Value>>,
}

/// A single e2e test fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    /// Unique identifier (used as test function name).
    pub id: String,
    /// Optional category (defaults to parent directory name).
    #[serde(default)]
    pub category: Option<String>,
    /// Human-readable description.
    pub description: String,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Skip directive.
    #[serde(default)]
    pub skip: Option<SkipDirective>,
    /// Named call config to use (references `[e2e.calls.<name>]`).
    /// When omitted, uses the default `[e2e.call]`.
    #[serde(default)]
    pub call: Option<String>,
    /// Input data passed to the function under test.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Optional mock HTTP response for testing HTTP clients.
    #[serde(default)]
    pub mock_response: Option<MockResponse>,
    /// List of assertions to check.
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    /// Source file path (populated during loading).
    #[serde(skip)]
    pub source: String,
}

impl Fixture {
    /// Returns true if this fixture requires a mock HTTP server.
    pub fn needs_mock_server(&self) -> bool {
        self.mock_response.is_some()
    }

    /// Returns true if the mock response uses streaming (SSE).
    pub fn is_streaming_mock(&self) -> bool {
        self.mock_response
            .as_ref()
            .and_then(|m| m.stream_chunks.as_ref())
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    /// Get the resolved category (explicit or from source directory).
    pub fn resolved_category(&self) -> String {
        self.category.clone().unwrap_or_else(|| {
            Path::new(&self.source)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string()
        })
    }
}

/// Skip directive for conditionally excluding fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipDirective {
    /// Languages to skip (empty means skip all).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Human-readable reason for skipping.
    #[serde(default)]
    pub reason: Option<String>,
}

impl SkipDirective {
    /// Check if this fixture should be skipped for a given language.
    pub fn should_skip(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.iter().any(|l| l == language)
    }
}

/// A single assertion in a fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    /// Assertion type (equals, contains, not_empty, error, etc.).
    #[serde(rename = "type")]
    pub assertion_type: String,
    /// Field path to access on the result (dot-separated).
    #[serde(default)]
    pub field: Option<String>,
    /// Expected value (string, number, bool, or array depending on type).
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Expected values (for contains_all, contains_any).
    #[serde(default)]
    pub values: Option<Vec<serde_json::Value>>,
    /// Method name to call on the result (for method_result assertions).
    #[serde(default)]
    pub method: Option<String>,
    /// Assertion check type for the method result (equals, is_true, is_false, greater_than_or_equal, count_min).
    #[serde(default)]
    pub check: Option<String>,
    /// Arguments to pass to the method call (for method_result assertions).
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

/// A group of fixtures sharing the same category.
#[derive(Debug, Clone)]
pub struct FixtureGroup {
    pub category: String,
    pub fixtures: Vec<Fixture>,
}

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

            // Try parsing as array first, then as single fixture
            let parsed: Vec<Fixture> = if content.trim_start().starts_with('[') {
                serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?
            } else {
                let single: Fixture = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                vec![single]
            };

            for mut fixture in parsed {
                fixture.source = relative.clone();
                fixtures.push(fixture);
            }
        }
    }
    Ok(())
}

/// Group fixtures by their resolved category.
pub fn group_fixtures(fixtures: &[Fixture]) -> Vec<FixtureGroup> {
    let mut groups: HashMap<String, Vec<Fixture>> = HashMap::new();
    for f in fixtures {
        groups.entry(f.resolved_category()).or_default().push(f.clone());
    }
    let mut result: Vec<FixtureGroup> = groups
        .into_iter()
        .map(|(category, fixtures)| FixtureGroup { category, fixtures })
        .collect();
    result.sort_by(|a, b| a.category.cmp(&b.category));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_with_mock_response() {
        let json = r#"{
            "id": "test_chat",
            "description": "Test chat",
            "call": "chat",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}]},
            "mock_response": {
                "status": 200,
                "body": {"choices": [{"message": {"content": "hello"}}]}
            },
            "assertions": [{"type": "not_error"}]
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
        assert_eq!(fixture.mock_response.unwrap().status, 200);
    }

    #[test]
    fn test_fixture_with_streaming_mock_response() {
        let json = r#"{
            "id": "test_stream",
            "description": "Test streaming",
            "input": {},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}, {"delta": " world"}]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(fixture.is_streaming_mock());
    }

    #[test]
    fn test_fixture_without_mock_response() {
        let json = r#"{
            "id": "test_no_mock",
            "description": "No mock",
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
    }
}
