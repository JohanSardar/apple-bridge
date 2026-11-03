use lab_world::{Error, Result, World};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
}
#[derive(Debug, Serialize)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
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
            detail: detail.into(),
        });
    }
}
pub type Handler = fn(&mut World, &Value, &mut Evidence) -> Result<()>;
#[derive(Debug, Default)]
pub struct Registry {
    handlers: BTreeMap<String, Handler>,
}
impl Registry {
    pub fn register(&mut self, name: &str, handler: Handler) {
        self.handlers.insert(name.into(), handler);
    }
    pub fn run(&self, scenario: &Scenario, world: &mut World) -> Evidence {
        let mut evidence = Evidence {
            scenario: scenario.id.clone(),
            passed: true,
            assertions: vec![],
        };
        if !scenario.levels.iter().any(|s| s == "sim") {
            evidence.check("sim_level", false, "unsupported level");
            return evidence;
        }
        for step in scenario
            .given
            .iter()
            .chain(&scenario.when)
            .chain(&scenario.then)
        {
            if let Err(error) = self.step(step, world, &mut evidence) {
                evidence.check("action", false, &error.to_string());
                break;
            }
        }
        for step in &scenario.cleanup {
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
        registry
    }
}
fn capture_truth(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let truth = world.control("/inspect", &Value::Null)?;
    let expected = truth
        .as_array()
        .ok_or_else(|| Error::Invalid("inspect array".into()))?;
    let response = world.query("records.query", &json!({"collections":[],"limit":10000}))?;
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
