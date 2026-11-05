use super::{DESCRIPTION, IDENTIFIER, ROLE, Result, Ui};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub(in crate::session) struct Node {
    pub(in crate::session) handle: i64,
    pub(super) attrs: Value,
    pub(super) nested: Vec<Node>,
    pub(super) nested_left: i64,
}

impl Node {
    pub(super) fn parse(value: &Value) -> Self {
        Self {
            handle: value["handle"].as_i64().unwrap_or(-1),
            attrs: value["attrs"].clone(),
            nested: value["nested"]["nodes"]
                .as_array()
                .map(|nodes| nodes.iter().map(Self::parse).collect())
                .unwrap_or_default(),
            nested_left: value["nested"]["budget_left"].as_i64().unwrap_or(0),
        }
    }

    pub(super) fn get(&self, attribute: &str) -> &str {
        self.attrs[attribute].as_str().unwrap_or_default()
    }
}

#[derive(Default, Clone)]
pub(super) struct Scan<'a> {
    pub(super) within: Option<i64>,
    pub(super) role: Option<&'a [&'a str]>,
    pub(super) identifier: Option<&'a [&'a str]>,
    pub(super) description: Option<&'a [&'a str]>,
    pub(super) attributes: &'a [&'a str],
    pub(super) budget: i64,
    pub(super) depth: Option<i64>,
    pub(super) menu_bar: bool,
    pub(super) nested_identifier: Option<&'a [&'a str]>,
    pub(super) nested_attributes: &'a [&'a str],
    pub(super) nested_budget: i64,
}

impl Ui<'_> {
    pub(super) fn scan(&self, scan: &Scan<'_>) -> Result<(Vec<Node>, i64)> {
        let mut matched = serde_json::Map::new();
        if let Some(role) = scan.role {
            matched.insert(ROLE.into(), json!(role));
        }
        if let Some(identifier) = scan.identifier {
            matched.insert(IDENTIFIER.into(), json!(identifier));
        }
        if let Some(description) = scan.description {
            matched.insert(DESCRIPTION.into(), json!(description));
        }
        let mut args = json!({
            "match": matched,
            "attributes": scan.attributes,
            "budget": scan.budget,
            "menu_bar": scan.menu_bar,
        });
        if let Some(within) = scan.within {
            args["within"] = json!(within);
        }
        if let Some(depth) = scan.depth {
            args["depth"] = json!(depth);
        }
        if scan.nested_budget > 0 {
            let mut nested = json!({"attributes": scan.nested_attributes, "budget": scan.nested_budget, "match": {}});
            if let Some(identifier) = scan.nested_identifier {
                nested["match"] = json!({IDENTIFIER: identifier});
            }
            args["nested"] = nested;
        }
        let answer = self.call("messages.ax_find", &args)?;
        let nodes = answer["nodes"]
            .as_array()
            .map(|nodes| nodes.iter().map(Node::parse).collect())
            .unwrap_or_default();
        Ok((nodes, answer["budget_left"].as_i64().unwrap_or(0)))
    }
}
