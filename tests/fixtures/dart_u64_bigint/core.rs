use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Wide(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WideRecord {
    pub value: u64,
    pub optional: Option<u64>,
    pub values: Vec<u64>,
    pub named: HashMap<String, u64>,
    pub wrapped: Wide,
    pub byte: u8,
    pub short: u16,
    pub count: u32,
    pub signed: i64,
}

pub fn scalar_for_case(index: u32) -> u64 {
    [0, (1 << 53) + 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX][index as usize]
}

pub fn echo_scalar(value: u64) -> u64 {
    value
}

pub fn echo_optional(value: Option<u64>) -> Option<u64> {
    value
}

pub fn echo_vector(values: Vec<u64>) -> Vec<u64> {
    values
}

pub fn echo_map(values: HashMap<String, u64>) -> HashMap<String, u64> {
    values
}

pub fn echo_newtype(value: Wide) -> Wide {
    value
}

pub fn echo_record(record: WideRecord) -> WideRecord {
    record
}

pub fn echo_signed(value: i64) -> i64 {
    value
}

pub fn echo_u32(value: u32) -> u32 {
    value
}
