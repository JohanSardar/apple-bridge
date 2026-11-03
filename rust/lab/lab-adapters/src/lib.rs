pub mod assertions;
pub mod blobs;
pub mod cleanup;
pub mod commands;
pub mod history;
pub mod http;
pub mod imports;
pub mod journey;
pub mod media;
pub mod poster;
pub mod queries;
pub mod replicas;
pub mod restart;
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
        let self_started = media::MEDIA_SCENARIOS.contains(&id) || id == "MSG-MEDIA-CLEANUP";
        if replicas != 0 && blobs::is_blob(id) {
            blobs::configure(world, id, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && journey::COMMAND_SCENARIOS.contains(&id) {
            replicas::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0
            && (history::SCENARIOS.contains(&id) || imports::SCENARIOS.contains(&id))
        {
            replicas::start_with(
                world,
                usize::try_from(replicas).unwrap_or(0),
                history::SCOPE,
                0,
            )?;
        } else if replicas != 0 && !PORTED.contains(&id) && !self_started {
            return Err(Error::Invalid(format!("replica world not ported for {id}")));
        }
        evidence.check("world_started", true, "gateway and SimApple peer ready");
        Ok(())
    });
    registry.register("queries", queries::queries);
    registry.register("blobs", blobs::blobs);
    registry.register("messages", history::messages);
    registry.register("messages_read", history::messages_read);
    registry.register("messages_truth", history::messages_truth);
    registry.register("contacts", imports::contacts);
    registry.register("notes", imports::notes);
    registry.register("usage", imports::usage);
    registry.register("seed", journey::seed);
    registry.register("converge", journey::converge);
    registry.register("media_variants", media::media_variants);
    registry.register("messages_commands", cleanup::messages_commands);
    registry.register("command_journey", journey::command_journey);
    registry
}
