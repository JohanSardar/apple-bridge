pub mod http;
pub mod queries;
use lab_runner::Registry;
use lab_world::Error;

const PORTED: [&str; 4] = [
    "CORE-QUERY-ROUNDTRIP",
    "CORE-QUERY-BLOB",
    "CORE-QUERY-SCOPE",
    "CORE-QUERY-DEADLINE",
];

#[must_use]
pub fn registry() -> Registry {
    let mut registry = Registry::standard();
    registry.register("world", |_, input, evidence| {
        let replicas = input["replicas"].as_u64().unwrap_or(0);
        if replicas != 0 && !PORTED.contains(&evidence.scenario.as_str()) {
            return Err(Error::Invalid(format!(
                "replica world not ported for {}",
                evidence.scenario
            )));
        }
        evidence.check("world_started", true, "gateway and SimApple peer ready");
        Ok(())
    });
    registry.register("queries", queries::queries);
    registry
}
