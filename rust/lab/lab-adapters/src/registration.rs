use lab_runner::{Registry, Scenario, supports_level};

pub fn load(bytes: &[u8]) -> Result<Scenario, String> {
    let mut scenario: Scenario = serde_saphyr::from_slice(bytes).map_err(|e| e.to_string())?;
    crate::prepare(&mut scenario);
    if scenario.when.is_empty() && crate::readmodels::SCENARIOS.contains(&scenario.id.as_str()) {
        scenario.when.push(serde_json::json!({"readmodels": {}}));
    }
    if scenario.when.is_empty()
        && let Some(step) = crate::implied_step(&scenario.id)
    {
        scenario.when.push(serde_json::json!({ step: {} }));
    }
    Ok(scenario)
}

fn step_names(scenario: &Scenario) -> Result<Vec<String>, String> {
    let mut names = vec![];
    for step in scenario
        .given
        .iter()
        .chain(&scenario.when)
        .chain(&scenario.then)
        .chain(&scenario.cleanup)
    {
        let map = step
            .as_object()
            .filter(|v| v.len() == 1)
            .ok_or_else(|| "step must have one action".to_owned())?;
        names.extend(map.keys().cloned());
    }
    Ok(names)
}

#[must_use]
pub fn stub(id: &str) -> bool {
    crate::contacts::STUB.contains(&id)
        || crate::mail::REAL_ONLY.contains(&id)
        || crate::iphone_real::STUB.contains(&id)
        || crate::shortcuts::real_only_stub(id)
}

pub fn classify(scenario: &Scenario, registry: &Registry) -> Result<&'static str, String> {
    if scenario.levels.is_empty() {
        return Err("no levels declared".into());
    }
    if stub(&scenario.id) {
        return Ok("stub");
    }
    if supports_level(scenario, "real")
        && (crate::iphone_real::handles(&scenario.id) || registry.has_real(&scenario.id))
    {
        return Ok("compiled");
    }
    if scenario.levels.iter().any(|level| level == "real")
        && !scenario.levels.iter().any(|level| level == "sim")
    {
        return Err("no real adapter registered".into());
    }
    if !supports_level(scenario, "sim") {
        return Err(format!("no runnable level in {:?}", scenario.levels));
    }
    let mut names = step_names(scenario)?;
    if names.iter().all(|name| name == "world") {
        names.push(scenario.id.clone());
    }
    let missing: Vec<&String> = names
        .iter()
        .filter(|name| name.as_str() != "world" && !registry.has_step(name))
        .collect();
    if missing.is_empty() {
        Ok("compiled")
    } else {
        Err(format!("unregistered steps {missing:?}"))
    }
}
