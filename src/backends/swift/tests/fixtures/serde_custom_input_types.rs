use serde::{Deserialize, Serialize};

mod digits {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }

    pub fn serialize<S: Serializer>(value: &u32, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictRead {
    #[serde(deserialize_with = "digits::deserialize")]
    pub value: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictWith {
    #[serde(with = "digits")]
    pub value: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AliasRead {
    #[serde(alias = "legacy", deserialize_with = "digits::deserialize")]
    pub value: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AliasWith {
    #[serde(alias = "legacy", with = "digits")]
    pub value: u32,
}
