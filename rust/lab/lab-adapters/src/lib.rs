pub mod assertions;
pub mod http;
pub mod journey;
pub mod queries;
pub mod replicas;
pub mod watch;
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
    registry.register("world", |world, input, evidence| {
        let replicas = input["replicas"].as_u64().unwrap_or(0);
        let id = evidence.scenario.as_str();
        if replicas != 0 && journey::COMMAND_SCENARIOS.contains(&id) {
            replicas::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && !PORTED.contains(&id) {
            return Err(Error::Invalid(format!("replica world not ported for {id}")));
        }
        evidence.check("world_started", true, "gateway and SimApple peer ready");
        Ok(())
    });
    registry.register("queries", queries::queries);
    registry.register("seed", journey::seed);
    registry.register("converge", journey::converge);
    registry.register("command_journey", journey::command_journey);
    registry
}
