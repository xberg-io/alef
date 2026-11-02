use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct CollisionRecord {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub extras: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
pub struct RequiredChild {
    pub required_name: String,
    pub required_kind: String,
}

#[derive(Deserialize, Serialize)]
pub struct PartlyDefaultedChild {
    pub required_name: String,
    #[serde(default)]
    pub required_kind: String,
}

#[derive(Deserialize, Serialize)]
pub struct DefaultedChild {
    #[serde(default)]
    pub required_name: String,
    #[serde(default)]
    pub required_kind: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ContainerDefaultedChild {
    pub required_name: String,
    pub required_kind: String,
}

#[derive(Deserialize, Serialize)]
pub struct OwnedRequiredChild {
    pub name: String,
}

#[derive(Deserialize, Serialize)]
pub struct OwnedDefaultedChild {
    #[serde(default)]
    pub name: String,
}
