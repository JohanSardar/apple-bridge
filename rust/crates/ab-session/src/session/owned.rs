use serde_json::Value;

pub(super) trait Probe {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String>;
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    pub code: String,
    pub key: Option<String>,
    pub message: Option<String>,
}

pub(super) struct Settled {
    pub disposition: &'static str,
    pub evidence: Value,
}

type PrepareCommand = fn(&dyn Probe, &Value) -> Result<Value, Refusal>;

pub(super) struct Driver {
    pub name: &'static str,
    pub prepare: fn(&dyn Probe, &Value, &Value) -> Result<Value, Refusal>,
    pub prepare_command: Option<PrepareCommand>,
    pub validate: fn(&dyn Probe, &Value, &Value) -> Result<(), Refusal>,
    pub write: fn(&dyn Probe, &str, &Value, &Value) -> Result<Value, String>,
    pub settle: fn(&str, &Value) -> Option<Settled>,
    pub resolved: fn(&dyn Probe, &Value) -> Option<Value>,
    pub reconcile_args: fn(&Value, &str) -> Value,
}

pub(super) fn refuse(code: &str) -> Refusal {
    Refusal {
        code: code.into(),
        key: None,
        message: None,
    }
}

pub(super) fn conflict(key: &str) -> Refusal {
    Refusal {
        code: "conflict".into(),
        key: (!key.is_empty()).then(|| key.to_owned()),
        message: None,
    }
}

pub(super) fn native_failure(code: &str) -> Refusal {
    match code {
        "unsupported" | "invalid" | "conflict" => refuse(code),
        _ => refuse("unavailable"),
    }
}

pub(super) fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, x)| b.get(key).is_some_and(|y| equal(x, y)))
        }
        _ => left == right,
    }
}

pub(super) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

pub(super) fn driver(source: &str) -> Option<&'static Driver> {
    match source {
        "reminders" => Some(&super::reminders::DRIVER),
        "calendar" => Some(&super::calendar::DRIVER),
        "contacts" => Some(&super::contacts::DRIVER),
        "notes" => Some(&super::notes::DRIVER),
        "messages" => Some(&super::messages::DRIVER),
        _ => None,
    }
}
