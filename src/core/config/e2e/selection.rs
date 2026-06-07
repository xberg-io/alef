use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Condition for auto-selecting a named call config when the fixture matches.
///
/// When a fixture does not specify `"call"`, the codegen normally uses the default
/// `[e2e.call]`.  A `SelectWhen` condition on a named call allows automatic routing
/// based on the fixture's id, category, tags, or input shape.  All set fields must
/// match (logical AND); a condition with no fields set never matches.
///
/// ```toml
/// [e2e.calls.batch_scrape]
/// select_when = { input_has = "batch_urls" }
///
/// [e2e.calls.crawl]
/// select_when = { category = "crawl" }
///
/// [e2e.calls.batch_crawl_stream]
/// select_when = { category = "stream", id_prefix = "batch_crawl_stream" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
pub struct SelectWhen {
    /// Match when the fixture's resolved category equals this string.
    #[serde(default)]
    pub category: Option<String>,
    /// Match when the fixture's id starts with this prefix.
    #[serde(default)]
    pub id_prefix: Option<String>,
    /// Match when the fixture's id matches this simple glob.
    ///
    /// Only `*` (matches any run of characters) is supported. Use `id_prefix`
    /// for plain prefix matches.
    #[serde(default)]
    pub id_glob: Option<String>,
    /// Match when the fixture's tags include this tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Match when the fixture's input object contains this key with a non-null value.
    #[serde(default)]
    pub input_has: Option<String>,
}

impl SelectWhen {
    /// Returns true when every set discriminator matches the fixture.
    ///
    /// A `SelectWhen` with all fields `None` returns `false` — at least one
    /// discriminator must be set for the condition to fire.
    pub fn matches(
        &self,
        fixture_id: &str,
        fixture_category: &str,
        fixture_tags: &[String],
        fixture_input: &serde_json::Value,
    ) -> bool {
        let any_set = self.category.is_some()
            || self.id_prefix.is_some()
            || self.id_glob.is_some()
            || self.tag.is_some()
            || self.input_has.is_some();
        if !any_set {
            return false;
        }
        if let Some(cat) = &self.category
            && cat.as_str() != fixture_category
        {
            return false;
        }
        if let Some(prefix) = &self.id_prefix
            && !fixture_id.starts_with(prefix.as_str())
        {
            return false;
        }
        if let Some(glob) = &self.id_glob
            && !glob_matches(glob, fixture_id)
        {
            return false;
        }
        if let Some(tag) = &self.tag
            && !fixture_tags.iter().any(|t| t == tag)
        {
            return false;
        }
        if let Some(key) = &self.input_has {
            let val = fixture_input.get(key.as_str()).unwrap_or(&serde_json::Value::Null);
            if val.is_null() {
                return false;
            }
        }
        true
    }
}

/// Minimal glob matcher supporting `*` (greedy any-run) only.
fn glob_matches(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 {
            if !text[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
        } else if idx + 1 == parts.len() && !pattern.ends_with('*') {
            return text[cursor..].ends_with(part);
        } else {
            match text[cursor..].find(part) {
                Some(pos) => cursor += pos + part.len(),
                None => return false,
            }
        }
    }
    true
}
