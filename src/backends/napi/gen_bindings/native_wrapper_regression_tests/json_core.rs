#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub payload: serde_json::Value,
}

pub fn json_control(value: serde_json::Value) -> serde_json::Value {
    value
}

pub fn echo_packet(value: Packet) -> Packet {
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

    pub fn echo_json(&self, value: serde_json::Value) -> serde_json::Value {
        let _ = self.state;
        value
    }

    pub async fn echo_json_async(&self, value: serde_json::Value) -> serde_json::Value {
        let _ = self.state;
        value
    }

    pub fn echo_json_static(value: serde_json::Value) -> serde_json::Value {
        value
    }
}
