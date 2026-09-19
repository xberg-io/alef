//! Assertion payloads shared by fixture consumers.

use super::AssertionSkip;
use serde::{Deserialize, Serialize};

/// A single assertion in a fixture.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Assertion {
    /// Assertion type (equals, contains, not_empty, error, etc.).
    #[serde(rename = "type")]
    pub assertion_type: String,
    /// Field path to access on the result (dot-separated).
    #[serde(default)]
    pub field: Option<String>,
    /// Expected JSON value, including explicit null; absence means no expected value.
    #[serde(
        default,
        deserialize_with = "deserialize_assertion_value",
        skip_serializing_if = "Option::is_none"
    )]
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
    /// Return type hint for C method_result codegen.
    ///
    /// Supported values:
    /// - `"string"` — the method returns a heap-allocated `char*` that must be
    ///   freed with `free()` after the assertion.  The generator emits
    ///   `char* _r = call(); assert(...); free(_r);`.
    ///
    /// Defaults to primitive integer dispatch when absent.
    #[serde(default)]
    pub return_type: Option<String>,
    /// Explicit acknowledgement that this assertion cannot be generated (see [`AssertionSkip`]).
    ///
    /// Absent means "must generate": if a backend drops this assertion because its field did not
    /// resolve, generation fails. ~keep
    #[serde(default)]
    pub skip: Option<AssertionSkip>,
}

impl Assertion {
    pub(crate) fn expected_values(&self) -> Vec<&serde_json::Value> {
        self.values
            .as_ref()
            .map(|values| values.iter().collect())
            .or_else(|| self.value.as_ref().map(|value| vec![value]))
            .unwrap_or_default()
    }
}

// Serde's Option visitor collapses explicit null and missing fields. Deserialize a
// present value directly; only the field default should supply None. ~keep
fn deserialize_assertion_value<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::Assertion;
    use serde_json::{Value, json};

    #[test]
    fn expected_values_preserves_null_and_plural_precedence() {
        let assertion: Assertion = serde_json::from_value(json!({"type": "equals", "value": null})).unwrap();
        assert_eq!(assertion.expected_values(), vec![&Value::Null]);
        let assertion: Assertion =
            serde_json::from_value(json!({"type": "contains_all", "value": null, "values": [false, null]})).unwrap();
        assert_eq!(assertion.expected_values(), vec![&json!(false), &Value::Null]);
    }
}
