pub mod safety;
use lab_world::{Error, RealConfig, Result, World, WorldConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::{
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub feature: String,
    pub levels: Vec<String>,
    #[serde(default)]
    pub given: Vec<Value>,
    #[serde(default)]
    pub when: Vec<Value>,
    #[serde(default)]
    pub then: Vec<Value>,
    #[serde(default)]
    pub cleanup: Vec<Value>,
    #[serde(default)]
    pub web: Option<Value>,
}
#[derive(Debug, Serialize)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct Evidence {
    pub scenario: String,
    pub passed: bool,
    pub assertions: Vec<Assertion>,
}
impl Evidence {
    pub fn check(&mut self, name: &str, passed: bool, detail: &str) {
        self.passed &= passed;
        self.assertions.push(Assertion {
            name: name.into(),
            passed,
            status: None,
            detail: detail.into(),
        });
    }
    pub fn unverified(&mut self, name: &str, detail: &str) {
        self.assertions.push(Assertion {
            name: name.into(),
            passed: false,
            status: Some("unverified".into()),
            detail: detail.into(),
        });
    }
}
pub type Handler = fn(&mut World, &Value, &mut Evidence) -> Result<()>;
pub type RealHandler = fn(&mut World, &mut Evidence) -> Result<()>;
pub type StandaloneHandler = fn(&mut Evidence) -> Result<()>;
#[derive(Debug, Default)]
pub struct Registry {
    handlers: BTreeMap<String, Handler>,
    real: BTreeMap<String, RealHandler>,
    simulated: BTreeSet<String>,
    standalone: BTreeMap<String, StandaloneHandler>,
}
#[must_use]
pub fn supports_level(scenario: &Scenario, level: &str) -> bool {
    scenario.levels.iter().any(|s| s == level)
        || (level == "sim" && scenario.levels.iter().any(|s| s == "replica"))
        || (level == "real"
            && matches!(
                scenario.id.as_str(),
                "MAC-REPLICA"
                    | "FILE-REPLICA"
                    | "WEB-RELAY-COMMAND"
                    | "WEB-RELAY-CONNECT"
                    | "WEB-RELAY-NATIVE-AGENTS"
                    | "WEB-RELAY-PUBLIC-AGENTS"
                    | "WEB-RELAY-REPLICA"
                    | "WEB-RELAY-REPLICA-BLOBS"
            )
            && scenario.levels.iter().any(|s| s == "replica"))
}
impl Registry {
    pub fn register(&mut self, name: &str, handler: Handler) {
        self.handlers.insert(name.into(), handler);
    }
    pub fn register_real(&mut self, id: &str, handler: RealHandler) {
        self.real.insert(id.into(), handler);
    }
    pub fn register_real_simulated(&mut self, id: &str, handler: RealHandler) {
        self.real.insert(id.into(), handler);
        self.simulated.insert(id.into());
    }
    pub fn register_standalone(&mut self, id: &str, handler: StandaloneHandler) {
        self.standalone.insert(id.into(), handler);
    }
    #[must_use]
    pub fn has_step(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }
    #[must_use]
    pub fn has_real(&self, id: &str) -> bool {
        self.real.contains_key(id) || self.standalone.contains_key(id)
    }
    fn real_blocked(&self, scenario: &Scenario) -> Option<Evidence> {
        let mut evidence = Evidence {
            scenario: scenario.id.clone(),
            passed: true,
            assertions: vec![],
        };
        if !supports_level(scenario, "real") {
            evidence.check("real_level", false, "unsupported level");
        } else if !self.has_real(&scenario.id) {
            evidence.check("real_adapter", false, "no real adapter registered");
        } else {
            return None;
        }
        Some(evidence)
    }
    pub fn run_real(&self, scenario: &Scenario, config: RealConfig) -> Result<Evidence> {
        if let Some(blocked) = self.real_blocked(scenario) {
            return Ok(blocked);
        }
        let mut evidence = Evidence {
            scenario: scenario.id.clone(),
            passed: true,
            assertions: vec![],
        };
        if let Some(handler) = self.standalone.get(&scenario.id) {
            if let Err(error) = handler(&mut evidence) {
                evidence.check("action", false, &error.to_string());
            }
            if evidence.assertions.is_empty() {
                evidence.check("assertions_present", false, "scenario made no assertions");
            }
            return Ok(evidence);
        }
        let mut world = if self.simulated.contains(&scenario.id) {
            World::start(WorldConfig {
                gateway: config.gateway,
                simapple: config.simapple,
                directory: config.directory,
                scope: config.scope,
            })?
        } else {
            World::start_real(config)?
        };
        if let Some(handler) = self.real.get(&scenario.id)
            && let Err(error) = handler(&mut world, &mut evidence)
        {
            evidence.check("action", false, &error.to_string());
        }
        if evidence.assertions.is_empty() {
            evidence.check("assertions_present", false, "scenario made no assertions");
        }
        Ok(evidence)
    }
    pub fn run(&self, scenario: &Scenario, world: &mut World) -> Evidence {
        let mut evidence = Evidence {
            scenario: scenario.id.clone(),
            passed: true,
            assertions: vec![],
        };
        if !supports_level(scenario, "sim") {
            evidence.check("sim_level", false, "unsupported level");
            return evidence;
        }
        let implicit = [Value::Object(
            std::iter::once((scenario.id.clone(), json!({}))).collect(),
        )];
        let declared = scenario
            .given
            .iter()
            .chain(&scenario.when)
            .chain(&scenario.then)
            .count();
        let steps: &[Value] = if declared == 0 { &implicit } else { &[] };
        for step in scenario
            .given
            .iter()
            .chain(&scenario.when)
            .chain(&scenario.then)
            .chain(steps)
        {
            if let Err(error) = self.step(step, world, &mut evidence) {
                evidence.check("action", false, &error.to_string());
                break;
            }
        }
        let finish = json!({ format!("finish:{}", scenario.id): {} });
        if self
            .handlers
            .contains_key(&format!("finish:{}", scenario.id))
            && let Err(error) = self.step(&finish, world, &mut evidence)
        {
            evidence.check("finish", false, &error.to_string());
        }
        for step in &scenario.cleanup {
            if step.get("world").is_some() {
                world.close();
                continue;
            }
            if let Err(error) = self.step(step, world, &mut evidence) {
                evidence.check("cleanup", false, &error.to_string());
            }
        }
        if evidence.assertions.is_empty() {
            evidence.check("assertions_present", false, "scenario made no assertions");
        }
        evidence
    }
    fn step(&self, step: &Value, world: &mut World, evidence: &mut Evidence) -> Result<()> {
        let map = step
            .as_object()
            .filter(|v| v.len() == 1)
            .ok_or_else(|| Error::Invalid("step must have one action".into()))?;
        let (name, input) = map
            .iter()
            .next()
            .ok_or_else(|| Error::Invalid("empty step".into()))?;
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| Error::Invalid(format!("unsupported action: {name}")))?;
        handler(world, input, evidence)
    }
    pub fn standard() -> Self {
        let mut registry = Self::default();
        registry.register("world", |_, input, _| {
            if input["replicas"].as_u64().unwrap_or(0) != 0 {
                return Err(Error::Invalid("replica worlds not yet implemented".into()));
            }
            Ok(())
        });
        registry.register("control", |world, input, _| {
            world.control(
                input["route"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("control route".into()))?,
                &input["input"],
            )?;
            Ok(())
        });
        registry.register("query", |world, input, evidence| {
            let operation = input["operation"]
                .as_str()
                .ok_or_else(|| Error::Invalid("query operation".into()))?;
            let actual = world.query(operation, &input["input"])?;
            if let Some(expected) = input.get("equals") {
                evidence.check(
                    operation,
                    actual == *expected,
                    "exact public response equality",
                );
            } else {
                evidence.check(operation, true, "public operation succeeded");
            }
            Ok(())
        });
        registry.register("restart_gateway", |world, _, _| world.restart_gateway());
        registry.register("capture_truth", capture_truth);
        registry.register("fault_trace", safety::gate);
        registry.register("protocol_gate", protocol_gate);
        registry.register("scope_gate", scope_gate);
        registry
    }
}
fn capture_truth(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let truth = world.control("/inspect", &Value::Null)?;
    let expected = truth
        .as_array()
        .ok_or_else(|| Error::Invalid("inspect array".into()))?;
    let response = world.query("records.query", &json!({"collection":"","limit":1000}))?;
    let records = response["records"]
        .as_array()
        .ok_or_else(|| Error::Invalid(format!("records array missing: {response}")))?;
    let matches = expected.iter().all(|native| {
        records.iter().any(|record| {
            record["collection"] == native["collection"]
                && record["data"] == native["data"]
                && record["deleted"] != true
        })
    });
    evidence.check(
        "capture_equals_truth",
        !expected.is_empty() && matches,
        "every synthetic native observation appears through public records.query",
    );
    Ok(())
}

fn protocol_snapshot(world: &World, ready: impl Fn(&Value) -> bool) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let protocol = world.control("/protocol", &Value::Null)?;
        if ready(&protocol) || Instant::now() >= deadline {
            return Ok(protocol);
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn protocol_gate(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    match input["check"].as_str() {
        Some("reopen-offers-committed") => {
            let protocol = protocol_snapshot(world, |p| {
                p["opens"].as_array().is_some_and(|o| o.len() >= 2)
            })?;
            let commits = protocol["commits"].as_array().cloned().unwrap_or_default();
            let opens = protocol["opens"].as_array().cloned().unwrap_or_default();
            let last = opens.last().cloned().unwrap_or(Value::Null);
            let offered = last["offered"].clone();
            let seen = usize::try_from(last["commits_seen"].as_u64().unwrap_or(0))
                .unwrap_or(0)
                .min(commits.len());
            let mut latest = BTreeMap::new();
            for commit in &commits[..seen] {
                latest.insert(
                    commit["source"].as_str().unwrap_or_default().to_owned(),
                    commit["checkpoint"].clone(),
                );
            }
            let ok = opens.len() >= 2
                && !latest.is_empty()
                && latest
                    .iter()
                    .all(|(source, checkpoint)| offered[source.as_str()] == *checkpoint);
            evidence.check(
                "reopen_offers_committed_checkpoint",
                ok,
                &format!(
                    "opens={} committed_before_open={seen} sources={}",
                    opens.len(),
                    latest.len()
                ),
            );
            Ok(())
        }
        Some("acked-data-visible") => {
            let protocol = protocol_snapshot(world, |p| {
                p["commits"].as_array().is_some_and(|c| !c.is_empty())
            })?;
            let acked = protocol["commits"]
                .as_array()
                .is_some_and(|c| !c.is_empty());
            let truth = world.control("/inspect", &Value::Null)?;
            let expected = truth.as_array().cloned().unwrap_or_default();
            let response = world.query("records.query", &json!({"collection":"","limit":1000}))?;
            let records = response["records"].as_array().cloned().unwrap_or_default();
            let visible = !expected.is_empty()
                && expected.iter().all(|native| {
                    records.iter().any(|record| {
                        record["collection"] == native["collection"]
                            && record["data"] == native["data"]
                            && record["deleted"] != true
                    })
                });
            evidence.check(
                "acked_data_visible",
                acked && visible,
                &format!("peer_acked={acked} visible_through_public_query={visible}"),
            );
            Ok(())
        }
        Some("stale-fence-rejected") => {
            let protocol = protocol_snapshot(world, |p| !p["stale"].is_null())?;
            let stale = protocol["stale"].clone();
            let key = stale["key"].as_str().unwrap_or_default();
            let collection = stale["collection"].as_str().unwrap_or_default();
            let superseded = stale["stale_fence"].as_u64().unwrap_or(0)
                < stale["current_fence"].as_u64().unwrap_or(0)
                && stale["stale_fence"].as_u64().unwrap_or(0) > 0;
            let reply = stale["reply"].as_str().unwrap_or_default();
            let response = world.query(
                "records.query",
                &json!({"collection":collection,"limit":1000}),
            )?;
            let visible = response["records"].as_array().is_some_and(|records| {
                records.iter().any(|record| record["data"] == stale["data"])
            });
            evidence.check(
                "stale_fence_rejected",
                superseded && !key.is_empty() && !stale["data"].is_null() && reply != "committed" && !visible,
                &format!("superseded={superseded} stale_reply={reply} visible_through_public_query={visible}"),
            );
            Ok(())
        }
        _ => Err(Error::Invalid("protocol_gate check".into())),
    }
}
fn scope_gate(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let field = |name: &str| {
        input[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Invalid(format!("scope_gate {name}")))
    };
    let (scope, inside, outside) = (field("scope")?, field("inside")?, field("outside")?);
    let narrow = world
        .gateway_cli(&["token", "create", "--scope", &scope])?
        .trim()
        .to_owned();
    let count = |token: Option<&str>, collection: &str| -> Result<Option<usize>> {
        let query = json!({"collection":collection,"limit":1000});
        let response = match token {
            Some(token) => world.query_with_token("records.query", &query, token),
            None => world.query("records.query", &query),
        };
        Ok(response
            .ok()
            .map(|value| value["records"].as_array().map_or(0, Vec::len)))
    };
    let outside_exists = count(None, &outside)?.is_some_and(|n| n > 0);
    let inside_visible = count(Some(&narrow), &inside)?.is_some_and(|n| n > 0);
    let leaked = count(Some(&narrow), &outside)?.unwrap_or(0);
    evidence.check(
        "scoped_token_hides_out_of_scope",
        outside_exists && inside_visible && leaked == 0,
        &format!(
            "out_of_scope_exists={outside_exists} in_scope_visible={inside_visible} leaked_records={leaked}"
        ),
    );
    Ok(())
}
