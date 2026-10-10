use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

pub const CORE: &str = r#"
pub use std::string::String;
#[derive(Clone, Default)]
pub struct Producer;
impl Producer {
    pub fn new() -> Self { Self }
    pub async fn unread_stream(&self) -> Result<impl futures::Stream<Item = Result<String, CoreError>> + Unpin, CoreError> {
        Ok(futures::stream::pending())
    }
}
#[derive(Debug)]
pub struct CoreError;
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("stream failed") }
}
impl std::error::Error for CoreError {}
pub async fn fetch() -> String {
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    "done".into()
}
"#;

pub const ADAPTER: &str = r#"
send_sync_types=['Producer']
[[crates.adapters]]
name='unread_stream'
pattern='streaming'
core_path='unread_stream'
owner_type='Producer'
item_type='String'
error_type='CoreError'
"#;

pub const WRAPPER: &str = r#"
fn core_error_to_py_err(error: sample_core::CoreError) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
}
#[pyfunction]
fn unread_stream(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    Producer::new().unread_stream(py)
}
"#;

pub fn with_stream(mut api: ApiSurface) -> ApiSurface {
    api.types.push(TypeDef {
        name: "Producer".into(),
        rust_path: "sample_core::Producer".into(),
        is_opaque: true,
        is_clone: true,
        has_default: true,
        methods: vec![
            MethodDef {
                name: "new".into(),
                return_type: TypeRef::Named("Producer".into()),
                ..Default::default()
            },
            MethodDef {
                name: "unread_stream".into(),
                receiver: Some(ReceiverKind::Ref),
                is_async: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    api
}
