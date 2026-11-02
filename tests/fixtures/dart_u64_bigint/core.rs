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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WideNested {
    pub record: WideRecord,
    pub optional: Option<WideRecord>,
    pub records: Vec<WideRecord>,
    pub named: HashMap<String, WideRecord>,
}

#[derive(Clone)]
pub struct Probe {
    calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

pub fn create_probe() -> Probe {
    Probe {
        calls: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
    }
}

pub fn probe_reset(probe: &Probe) {
    probe.calls.store(0, std::sync::atomic::Ordering::SeqCst);
}

pub fn probe_calls(probe: &Probe) -> u32 {
    probe.calls.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn checked_newtype(probe: &Probe, value: Wide) -> Wide {
    probe.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    value
}

pub fn checked_record(probe: &Probe, value: WideRecord) -> WideRecord {
    probe.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    value
}

pub fn checked_nested(probe: &Probe, value: WideNested) -> WideNested {
    probe.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    value
}
