use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leaf {
    pub wide: u64,
    pub optional_wide: Option<u64>,
    pub signed: i64,
    pub narrow: u32,
    pub word: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub leaf: Leaf,
    pub optional: Option<Leaf>,
    pub leaves: Vec<Leaf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeConfig {
    pub limit: u64,
}

pub fn echo_u64(value: u64) -> u64 {
    value
}
pub fn echo_i64(value: i64) -> i64 {
    value
}
pub fn echo_u32(value: u32) -> u32 {
    value
}
pub fn echo_usize(value: usize) -> usize {
    value
}
pub fn configure(config: ProbeConfig) -> u64 {
    config.limit
}
