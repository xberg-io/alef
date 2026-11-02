use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessorOptions {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, alias = "only")]
    pub enabled_processors: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_processors: Option<Vec<String>>,
    #[serde(skip, alias = "old_enabled_set")]
    pub enabled_set: Option<HashSet<String>>,
    #[serde(skip, alias = "old_disabled_set")]
    pub disabled_set: Option<HashSet<String>>,
}

fn default_true() -> bool {
    true
}
