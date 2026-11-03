pub mod assertions;
pub mod blobs;
pub mod cleanup;
pub mod commands;
pub mod files_native;
pub mod frontdoor;
pub mod history;
pub mod http;
pub mod imports;
pub mod iphone;
pub mod iphone_gateway;
pub mod iphone_push;
pub mod iphone_real;
pub mod journey;
pub mod labcore;
pub mod live;
pub mod mac;
pub mod mac_real;
pub mod media;
pub mod messages_a;
pub mod packaging;
pub mod phone;
pub mod poster;
pub mod queries;
pub mod readmodels;
pub mod relay;
pub mod relay_gated;
pub mod relay_service;
pub mod relay_tls;
pub mod reminders;
pub mod replicas;
pub mod restart;
pub mod robust;
pub mod search;
pub mod shortcuts;
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
        let self_started = media::MEDIA_SCENARIOS.contains(&id)
            || id == "MSG-MEDIA-CLEANUP"
            || readmodels::SCENARIOS.contains(&id);
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
        } else if replicas != 0 && messages_a::SCENARIOS.contains(&id) {
            messages_a::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && labcore::SCENARIOS.contains(&id) {
            labcore::start_world(world, id, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && robust::SCENARIOS.contains(&id) {
            replicas::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && !PORTED.contains(&id) && !self_started {
            return Err(Error::Invalid(format!("replica world not ported for {id}")));
        }
        evidence.check("world_started", true, "gateway and SimApple peer ready");
        Ok(())
    });
    registry.register("queries", queries::queries);
    registry.register("readmodels", readmodels::readmodels);
    registry.register("blobs", blobs::blobs);
    registry.register("messages", history::messages);
    registry.register("messages_read", messages_a::messages_read);
    registry.register("messages_truth", history::messages_truth);
    registry.register("contacts", imports::contacts);
    registry.register("notes", imports::notes);
    registry.register("usage", imports::usage);
    registry.register("seed", labcore::seed);
    registry.register("converge", labcore::converge);
    registry.register("connect", labcore::connect);
    registry.register("partition", labcore::partition);
    registry.register("heal", labcore::heal);
    registry.register("resumed", labcore::resumed);
    registry.register("truth_multiplicity", labcore::truth_multiplicity);
    registry.register("sse_resume", labcore::sse_resume_all);
    registry.register("sse_idle", labcore::sse_idle_all);
    registry.register("stale_frame", labcore::stale_frame);
    registry.register("stale_fence_rejected", labcore::stale_fence_rejected);
    registry.register("load_rejected", labcore::load_rejected);
    registry.register("relay_service", relay::relay_service);
    registry.register("seed", journey::seed);
    registry.register("converge", journey::converge);
    registry.register("media_variants", media::media_variants);
    registry.register("messages_commands", cleanup::messages_commands);
    registry.register("files_native", files_native::files_native);
    registry.register("robustness", robust::dispatch);
    registry.register("command_journey", journey::command_journey);
    registry.register("files_transform", shortcuts::files_transform);
    registry.register("shortcuts_actions", shortcuts::shortcuts_actions);
    registry.register("mac", mac::mac);
    registry
}
