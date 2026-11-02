#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum UnitInput {
    Text(String),
    #[default]
    Empty,
}

pub async fn consume_unit(value: UnitInput) {
    let _ = value;
}

pub fn json_control(value: serde_json::Value) -> serde_json::Value {
    value
}

pub fn ping() -> u32 {
    42
}

#[derive(Clone, Default)]
pub struct Engine {
    state: usize,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn consume_unit(&self, value: UnitInput) {
        let _ = (self.state, value);
    }

    pub async fn consume_static(value: UnitInput) {
        let _ = value;
    }
}
