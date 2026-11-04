use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct ScenarioRef {
    pub status: String,
    pub reason: String,
    pub id: String,
    pub levels: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Limit {
    pub scenario: String,
    pub level: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Feature {
    pub notes: String,
    pub limits: Vec<Limit>,
    pub id: String,
    pub title: String,
    pub requirements: Vec<String>,
    pub milestone: String,
    pub modules: Vec<String>,
    pub depends_on: Vec<String>,
    pub status: String,
    pub scenarios: Vec<ScenarioRef>,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub levels: BTreeMap<String, String>,
    pub statuses: Vec<String>,
    pub requirements: BTreeMap<String, String>,
    pub features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
pub struct Evidence {
    pub passed: Option<bool>,
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
