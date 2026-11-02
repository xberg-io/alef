pub fn json_borrowed(value: &serde_json::Value) -> &serde_json::Value {
    value
}

pub fn optional_json(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    value
}

pub fn ping() -> u32 {
    42
}

#[derive(Clone, Default)]
pub struct Engine {
    value: serde_json::Value,
    optional: Option<serde_json::Value>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn json_borrowed(&self) -> &serde_json::Value {
        &self.value
    }

    pub fn optional_json(&self) -> Option<serde_json::Value> {
        self.optional.clone()
    }

    pub fn borrowed_optional_json(&self) -> &Option<serde_json::Value> {
        &self.optional
    }

    pub fn optional_json_static(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
        value
    }
}
