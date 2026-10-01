//! JSON Schema generation for `alef.toml`.

use std::path::Path;

use anyhow::{Context, Result, bail};
use schemars::schema_for;
use serde_json::{Value, json};

use super::NewAlefConfig;

pub const DEFAULT_SCHEMA_PATH: &str = "schemas/alef.schema.json";
const SCHEMA_TITLE: &str = "Alef configuration";
const SCHEMA_DESCRIPTION: &str = "JSON Schema for the JSON representation of alef.toml.";

/// The root keys [`alef_config_schema`] stamps with the alef version, as opposed to the keys
/// `schemars` derives from [`NewAlefConfig`] itself.
///
/// A difference confined to these keys means the vendored copy still describes exactly the same
/// `alef.toml` surface -- an editor validating against it gives the same answers -- and a
/// difference outside them means it does not. [`classify_alef_config_schema`] is the only place
/// that distinction is drawn, so `alef schema --check` and `alef verify` cannot come to disagree
/// about what "stale" means for the same file. ~keep
const VERSION_STAMP_KEYS: &[&str] = &["$id", "version", "x-alef-version"];

/// How a schema file on disk differs from the one the running alef renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDrift {
    /// The file is byte-identical to the rendered schema.
    None,
    /// The bytes differ, but only in [`VERSION_STAMP_KEYS`] or in JSON formatting: the described
    /// `alef.toml` surface is unchanged, so editor validation against this copy is still correct.
    /// Carries the on-disk `x-alef-version` (absent for a file that never had one) and the
    /// expected one; the two are equal when the difference is formatting alone.
    SurfaceUnchanged {
        found_version: Option<String>,
        expected_version: String,
    },
    /// The described `alef.toml` surface itself differs -- or the file is not parseable as the
    /// JSON object a schema must be, which is indistinguishable from that for every purpose a
    /// consumer has for the file.
    Shape,
}

impl SchemaDrift {
    /// True when the vendored copy describes a different `alef.toml` surface than this alef.
    #[must_use]
    pub fn describes_a_different_surface(&self) -> bool {
        matches!(self, Self::Shape)
    }
}

/// Build the versioned JSON Schema for `alef.toml`.
pub fn alef_config_schema(version: &str) -> Result<Value> {
    let mut schema =
        serde_json::to_value(schema_for!(NewAlefConfig)).context("failed to serialize Alef config schema")?;
    let object = schema
        .as_object_mut()
        .context("schemars produced a non-object root schema")?;

    object.insert(
        "$id".to_string(),
        json!(format!(
            "https://github.com/xberg-io/alef/releases/download/v{version}/alef.schema.json"
        )),
    );
    object.insert("title".to_string(), json!(SCHEMA_TITLE));
    object.insert("description".to_string(), json!(SCHEMA_DESCRIPTION));
    object.insert("version".to_string(), json!(version));
    object.insert("x-alef-version".to_string(), json!(version));

    Ok(schema)
}

/// Render the versioned schema as pretty JSON with a trailing newline.
pub fn render_alef_config_schema(version: &str) -> Result<String> {
    let schema = alef_config_schema(version)?;
    let mut rendered = serde_json::to_string_pretty(&schema).context("failed to render Alef config schema")?;
    rendered.push('\n');
    Ok(rendered)
}

/// Write the schema file, creating parent directories as needed.
pub fn write_alef_config_schema(path: &Path, version: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create schema directory {}", parent.display()))?;
    }
    let rendered = render_alef_config_schema(version)?;
    std::fs::write(path, rendered).with_context(|| format!("failed to write schema {}", path.display()))
}

/// Classify how the schema file at `path` differs from the one this alef renders for `version`.
///
/// Errors only when the file cannot be read at all (missing, unreadable); a file that exists but
/// holds something other than a JSON object is [`SchemaDrift::Shape`], not an error -- the
/// caller's remedy is the same regeneration either way.
pub fn classify_alef_config_schema(path: &Path, version: &str) -> Result<SchemaDrift> {
    let expected = render_alef_config_schema(version)?;
    let actual = std::fs::read_to_string(path).with_context(|| format!("failed to read schema {}", path.display()))?;
    if actual == expected {
        return Ok(SchemaDrift::None);
    }
    let (Ok(actual_value), Ok(expected_value)) = (
        serde_json::from_str::<Value>(&actual),
        serde_json::from_str::<Value>(&expected),
    ) else {
        return Ok(SchemaDrift::Shape);
    };
    let found_version = actual_value
        .get("x-alef-version")
        .and_then(Value::as_str)
        .map(str::to_string);
    if strip_version_stamp(&actual_value) == strip_version_stamp(&expected_value) {
        return Ok(SchemaDrift::SurfaceUnchanged {
            found_version,
            expected_version: version.to_string(),
        });
    }
    Ok(SchemaDrift::Shape)
}

/// `schema` with every [`VERSION_STAMP_KEYS`] entry removed, or `None` when it is not an object.
fn strip_version_stamp(schema: &Value) -> Option<serde_json::Map<String, Value>> {
    let mut object = schema.as_object()?.clone();
    for key in VERSION_STAMP_KEYS {
        object.remove(*key);
    }
    Some(object)
}

/// Verify that an existing schema file matches the generated schema.
///
/// Byte-exact by design: this is the command a release procedure runs over a copy it also
/// regenerates, so any difference at all -- version stamp included -- is a difference it must
/// report. `alef verify` reads the same [`classify_alef_config_schema`] verdict and applies a
/// looser policy to a *consumer's* vendored copy, which it does not regenerate. ~keep
pub fn check_alef_config_schema(path: &Path, version: &str) -> Result<()> {
    if classify_alef_config_schema(path, version)? != SchemaDrift::None {
        bail!(
            "{} is stale; regenerate it with `alef schema --output {}`",
            path.display(),
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_schema(dir: &Path, version: &str) -> std::path::PathBuf {
        let path = dir.join("alef.schema.json");
        write_alef_config_schema(&path, version).expect("schema writes");
        path
    }

    /// Control: the copy the running alef would write is not drift.
    #[test]
    fn current_schema_classifies_as_no_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_schema(dir.path(), "1.2.3");

        assert_eq!(
            classify_alef_config_schema(&path, "1.2.3").expect("classification succeeds"),
            SchemaDrift::None
        );
    }

    /// An alef version bump alone must be distinguishable from a config-surface change: it is the
    /// only difference a consumer's vendored copy picks up on the overwhelming majority of
    /// upgrades, and it changes no answer their editor gives about `alef.toml`. ~keep
    #[test]
    fn version_bump_alone_classifies_as_version_stamp_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_schema(dir.path(), "1.2.3");

        let drift = classify_alef_config_schema(&path, "1.2.4").expect("classification succeeds");

        assert_eq!(
            drift,
            SchemaDrift::SurfaceUnchanged {
                found_version: Some("1.2.3".to_string()),
                expected_version: "1.2.4".to_string(),
            }
        );
        assert!(!drift.describes_a_different_surface());
    }

    /// A copy reserialized by some other JSON tool differs byte-wise while describing the exact
    /// same surface. It must not be classified as a surface change -- that would fail a
    /// consumer's verification for a whitespace difference. ~keep
    #[test]
    fn reformatted_schema_classifies_as_surface_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.schema.json");
        let schema = alef_config_schema("1.2.3").expect("schema generation succeeds");
        std::fs::write(&path, serde_json::to_string(&schema).expect("compact renders")).expect("writes");

        assert_eq!(
            classify_alef_config_schema(&path, "1.2.3").expect("classification succeeds"),
            SchemaDrift::SurfaceUnchanged {
                found_version: Some("1.2.3".to_string()),
                expected_version: "1.2.3".to_string(),
            }
        );
    }

    #[test]
    fn changed_config_surface_classifies_as_shape_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.schema.json");
        let mut schema = alef_config_schema("1.2.3").expect("schema generation succeeds");
        schema
            .as_object_mut()
            .expect("schema root is an object")
            .insert("properties".to_string(), json!({}));
        std::fs::write(&path, serde_json::to_string_pretty(&schema).expect("renders")).expect("writes");

        let drift = classify_alef_config_schema(&path, "1.2.3").expect("classification succeeds");

        assert_eq!(drift, SchemaDrift::Shape);
        assert!(drift.describes_a_different_surface());
    }

    #[test]
    fn unparseable_schema_classifies_as_shape_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.schema.json");
        std::fs::write(&path, "not json at all\n").expect("writes");

        assert_eq!(
            classify_alef_config_schema(&path, "1.2.3").expect("classification succeeds"),
            SchemaDrift::Shape
        );
    }

    /// `alef schema --check` stays byte-exact across both drift kinds, so the release procedure
    /// that regenerates the file keeps failing on a version stamp it forgot to refresh. ~keep
    #[test]
    fn schema_check_fails_on_a_version_stamp_difference_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_schema(dir.path(), "1.2.3");

        let error = check_alef_config_schema(&path, "1.2.4").expect_err("stale schema should fail");

        assert!(
            error.to_string().contains("is stale"),
            "expected stale schema error, got: {error}"
        );
    }
}
