#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strict {
    // Non-snake-case source spelling exercises canonical exclusion keys. ~keep
    #[allow(non_snake_case)]
    pub cacheDir: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AlefWireKey {
    #[error("broken")]
    Broken,
}

pub struct AlefWireKey1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AlefWireKey2 {
    pub label: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Available,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub mode: Mode,
}
