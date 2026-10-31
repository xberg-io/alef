#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strict {
    pub kept: String,
    pub omitted: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AlefWireKey {
    pub label: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictOwner {
    pub payload: AlefWireKey,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AliasOwner {
    #[serde(alias = "legacy_payload")]
    pub payload: AlefWireKey,
}
