use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<BTreeMap<String, String>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioRef {
    #[serde(default, deserialize_with = "text")]
    pub status: String,
    #[serde(default, deserialize_with = "text")]
    pub reason: String,
    #[serde(default, deserialize_with = "text")]
    pub id: String,
    #[serde(default, deserialize_with = "list")]
    pub levels: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    #[serde(default, deserialize_with = "text")]
    pub scenario: String,
    #[serde(default, deserialize_with = "text")]
    pub level: String,
    #[serde(default, deserialize_with = "text")]
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    #[serde(default, deserialize_with = "text")]
    pub notes: String,
    #[serde(default, deserialize_with = "list")]
    pub limits: Vec<Limit>,
    #[serde(default, deserialize_with = "text")]
    pub id: String,
    #[serde(default, deserialize_with = "text")]
    pub title: String,
    #[serde(default, deserialize_with = "list")]
    pub requirements: Vec<String>,
    #[serde(default, deserialize_with = "text")]
    pub milestone: String,
    #[serde(default, deserialize_with = "list")]
    pub modules: Vec<String>,
    #[serde(default, deserialize_with = "list")]
    pub depends_on: Vec<String>,
    #[serde(default, deserialize_with = "text")]
    pub status: String,
    #[serde(default, deserialize_with = "list")]
    pub scenarios: Vec<ScenarioRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    #[serde(default)]
    pub version: i64,
    #[serde(default, deserialize_with = "map")]
    pub levels: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "list")]
    pub statuses: Vec<String>,
    #[serde(default, deserialize_with = "map")]
    pub status_rules: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "map")]
    pub requirements: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "list")]
    pub features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
pub struct Evidence {
    pub passed: Option<bool>,
    pub git_sha: Option<String>,
    pub at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LevelResult {
    pub level: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct ScenarioView {
    pub status: String,
    pub reason: String,
    pub id: String,
    pub file: String,
    pub results: Vec<LevelResult>,
}

#[derive(Debug, Clone)]
pub struct FeatureView {
    pub feature: Feature,
    pub views: Vec<ScenarioView>,
}
