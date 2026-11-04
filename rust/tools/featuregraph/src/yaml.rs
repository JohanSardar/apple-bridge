use crate::model::{Feature, Graph, Limit, ScenarioRef};
use std::collections::BTreeMap;
use yaml_rust2::{Yaml, YamlLoader};

type Fields<'a> = Vec<(String, &'a Yaml)>;

fn scalar(value: &Yaml) -> Result<String, String> {
    match value {
        Yaml::String(text) | Yaml::Real(text) => Ok(text.clone()),
        Yaml::Integer(number) => Ok(number.to_string()),
        Yaml::Boolean(flag) => Ok(flag.to_string()),
        Yaml::Null => Ok(String::new()),
        other => Err(format!("cannot decode {other:?} as a string")),
    }
}

fn fields<'a>(value: &'a Yaml, known: &[&str], what: &str) -> Result<Fields<'a>, String> {
    let Yaml::Hash(map) = value else {
        return Err(format!("{what} must be a mapping"));
    };
    let mut out = Vec::new();
    for (key, item) in map {
        let key = scalar(key)?;
        if !known.contains(&key.as_str()) {
            return Err(format!("field {key} not found in {what}"));
        }
        out.push((key, item));
    }
    Ok(out)
}

fn strings(value: &Yaml) -> Result<Vec<String>, String> {
    match value {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Array(items) => items.iter().map(scalar).collect(),
        other => Err(format!("cannot decode {other:?} as a list")),
    }
}

fn string_map(value: &Yaml) -> Result<BTreeMap<String, String>, String> {
    match value {
        Yaml::Null => Ok(BTreeMap::new()),
        Yaml::Hash(map) => map
            .iter()
            .map(|(key, item)| Ok((scalar(key)?, scalar(item)?)))
            .collect(),
        other => Err(format!("cannot decode {other:?} as a mapping")),
    }
}

fn list<T>(value: &Yaml, convert: fn(&Yaml) -> Result<T, String>) -> Result<Vec<T>, String> {
    match value {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Array(items) => items.iter().map(convert).collect(),
        other => Err(format!("cannot decode {other:?} as a list")),
    }
}

fn scenario(value: &Yaml) -> Result<ScenarioRef, String> {
    let mut out = ScenarioRef::default();
    for (key, item) in fields(value, &["status", "reason", "id", "levels"], "scenario")? {
        match key.as_str() {
            "status" => out.status = scalar(item)?,
            "reason" => out.reason = scalar(item)?,
            "id" => out.id = scalar(item)?,
            _ => out.levels = strings(item)?,
        }
    }
    Ok(out)
}

fn limit(value: &Yaml) -> Result<Limit, String> {
    let mut out = Limit::default();
    for (key, item) in fields(value, &["scenario", "level", "reason"], "limit")? {
        match key.as_str() {
            "scenario" => out.scenario = scalar(item)?,
            "level" => out.level = scalar(item)?,
            _ => out.reason = scalar(item)?,
        }
    }
    Ok(out)
}

const FEATURE_FIELDS: [&str; 10] = [
    "notes",
    "limits",
    "id",
    "title",
    "requirements",
    "milestone",
    "modules",
    "depends_on",
    "status",
    "scenarios",
];

fn feature(value: &Yaml) -> Result<Feature, String> {
    let mut out = Feature::default();
    for (key, item) in fields(value, &FEATURE_FIELDS, "feature")? {
        match key.as_str() {
            "notes" => out.notes = scalar(item)?,
            "limits" => out.limits = list(item, limit)?,
            "id" => out.id = scalar(item)?,
            "title" => out.title = scalar(item)?,
            "requirements" => out.requirements = strings(item)?,
            "milestone" => out.milestone = scalar(item)?,
            "modules" => out.modules = strings(item)?,
            "depends_on" => out.depends_on = strings(item)?,
            "status" => out.status = scalar(item)?,
            _ => out.scenarios = list(item, scenario)?,
        }
    }
    Ok(out)
}

pub fn parse_graph(raw: &str) -> Result<Graph, String> {
    let documents = YamlLoader::load_from_str(raw).map_err(|e| e.to_string())?;
    let document = documents.first().ok_or("empty document")?;
    let mut graph = Graph::default();
    let known = [
        "version",
        "levels",
        "statuses",
        "status_rules",
        "requirements",
        "features",
    ];
    for (key, item) in fields(document, &known, "graph")? {
        match key.as_str() {
            "levels" => graph.levels = string_map(item)?,
            "statuses" => graph.statuses = strings(item)?,
            "requirements" => graph.requirements = string_map(item)?,
            "features" => graph.features = list(item, feature)?,
            _ => {}
        }
    }
    Ok(graph)
}

#[derive(Debug, Default)]
pub struct Head {
    pub id: String,
    pub feature: String,
    pub levels: Vec<String>,
}

#[must_use]
pub fn parse_head(raw: &str) -> Option<Head> {
    let documents = YamlLoader::load_from_str(raw).ok()?;
    let Yaml::Hash(map) = documents.first()? else {
        return None;
    };
    let mut head = Head::default();
    for (key, item) in map {
        match scalar(key).ok()?.as_str() {
            "id" => head.id = scalar(item).ok()?,
            "feature" => head.feature = scalar(item).ok()?,
            "levels" => head.levels = strings(item).ok()?,
            _ => {}
        }
    }
    Some(head)
}
