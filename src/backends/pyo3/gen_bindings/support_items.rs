use crate::codegen::builder::RustFileBuilder;

pub(super) fn add_py_visitor_ref(builder: &mut RustFileBuilder) {
    builder.add_item(
        r#"
/// Wrapper for trait visitor types (`Py<PyAny>`) that implements Clone.
///
/// `Py<PyAny>` is not Clone. This wrapper uses `Arc<Py<PyAny>>` internally for cheap cloning.
/// The .inner field is public for compatibility with generated code that needs to access
/// the underlying `Py<PyAny>` for trait dispatch.
#[derive(Debug)]
pub struct PyVisitorRef {
    pub inner: std::sync::Arc<pyo3::Py<pyo3::PyAny>>,
}

impl Clone for PyVisitorRef {
    fn clone(&self) -> Self {
        PyVisitorRef {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

impl From<pyo3::Py<pyo3::PyAny>> for PyVisitorRef {
    fn from(visitor: pyo3::Py<pyo3::PyAny>) -> Self {
        PyVisitorRef {
            inner: std::sync::Arc::new(visitor),
        }
    }
}

impl<'a, 'py> pyo3::FromPyObject<'a, 'py> for PyVisitorRef {
    type Error = pyo3::PyErr;

    fn extract(ob: pyo3::Borrowed<'a, 'py, pyo3::PyAny>) -> pyo3::PyResult<Self> {
        Ok(PyVisitorRef {
            inner: std::sync::Arc::new(ob.to_owned().unbind()),
        })
    }
}

impl<'py> pyo3::conversion::IntoPyObject<'py> for PyVisitorRef {
    type Target = pyo3::PyAny;
    type Output = pyo3::Bound<'py, pyo3::PyAny>;
    type Error = std::convert::Infallible;

    fn into_pyobject(self, py: pyo3::Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok((*self.inner).bind(py).clone())
    }
}
"#,
    );
}

pub(super) fn add_json_helpers(builder: &mut RustFileBuilder) {
    builder.add_item(
        r#"
mod alef_json_str {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;
    pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        Ok(match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

mod alef_json_str_opt {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Option<Value> = Option::deserialize(deserializer)?;
        Ok(v.and_then(|val| match val {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(other.to_string()),
        }))
    }
}
"#,
    );
}
