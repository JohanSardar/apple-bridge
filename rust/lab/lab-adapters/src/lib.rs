pub mod assertions;
pub mod blobs;
pub mod calendar;
pub mod calendar_fixtures;
pub mod calls;
pub mod cleanup;
pub mod commands;
pub mod contacts_list;
pub mod filecmd;
pub mod filerep;
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
pub mod maps;
pub mod media;
pub mod messages_a;
pub mod messages_b;
pub mod native;
pub mod packaging;
pub mod phone;
pub mod pkginstall;
pub mod poster;
pub mod queries;
pub mod readmodels;
pub mod relay;
pub mod relay_certs;
pub mod relay_gated;
pub mod relay_service;
pub mod relay_tls;
pub mod reminders;
pub mod replicas;
pub mod restart;
pub mod robust;
pub mod safari;
pub mod search;
pub mod shortcuts;
pub mod usage;
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
#[allow(clippy::too_many_lines)]
pub fn registry() -> Registry {
    let mut registry = Registry::standard();
    registry.register("world", |world, input, evidence| {
        let replicas = input["replicas"].as_u64().unwrap_or(0);
        let id = evidence.scenario.as_str();
        let self_started = media::MEDIA_SCENARIOS.contains(&id)
            || id == "MSG-MEDIA-CLEANUP"
            || readmodels::SCENARIOS.contains(&id)
            || calls::NATIVE_SCENARIOS.contains(&id)
            || usage::NATIVE_SCENARIOS.contains(&id);
        if replicas != 0 && blobs::is_blob(id) {
            blobs::configure(world, id, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && calls::REPLICA_SCENARIOS.contains(&id) {
            replicas::start_with(
                world,
                usize::try_from(replicas).unwrap_or(0),
                calls::SCOPE,
                0,
            )?;
        } else if replicas != 0 && journey::COMMAND_SCENARIOS.contains(&id) {
            replicas::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0
            && (history::SCENARIOS.contains(&id)
                || imports::SCENARIOS.contains(&id)
                || usage::SCENARIOS.contains(&id))
        {
            replicas::start_with(
                world,
                usize::try_from(replicas).unwrap_or(0),
                history::SCOPE,
                0,
            )?;
        } else if let (true, Some(scope)) = (replicas != 0, messages_b::scope(id)) {
            replicas::start_with(world, usize::try_from(replicas).unwrap_or(0), scope, 0)?;
        } else if replicas != 0 && messages_a::commands::ALL.contains(&id) {
            replicas::start_with(
                world,
                usize::try_from(replicas).unwrap_or(0),
                messages_a::commands::SCOPE,
                0,
            )?;
        } else if replicas != 0 && messages_a::SCENARIOS.contains(&id) {
            messages_a::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && labcore::SCENARIOS.contains(&id) {
            labcore::start_world(world, id, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && robust::SCENARIOS.contains(&id) {
            replicas::start(world, usize::try_from(replicas).unwrap_or(0))?;
        } else if replicas != 0 && contacts_list::SCENARIOS.contains(&id) {
            replicas::start_with(
                world,
                usize::try_from(replicas).unwrap_or(0),
                contacts_list::SCOPE,
                0,
            )?;
        } else if replicas != 0 && calendar::NATIVE_ONLY.contains(&id) {
        } else if replicas != 0 && id == "CAL-STORE-READ" {
            replicas::start_with(world, 1, calendar::STORE_SCOPE, 0)?;
        } else if replicas != 0 && calendar::SCENARIOS.contains(&id) {
            replicas::start_with(world, 1, calendar::SCOPE, 0)?;
        } else if replicas != 0 && !PORTED.contains(&id) && !self_started {
            return Err(Error::Invalid(format!("replica world not ported for {id}")));
        }
        evidence.check("world_started", true, "gateway and SimApple peer ready");
        Ok(())
    });
    registry.register("queries", |world, input, evidence| {
        if maps::SIM.contains(&evidence.scenario.as_str()) {
            maps::sim(world, evidence)
        } else {
            queries::queries(world, input, evidence)
        }
    });
    registry.register("file_commands", filecmd::commands);
    for id in maps::REAL {
        registry.register_real(id, maps::real);
    }
    for id in filecmd::REAL {
        registry.register_standalone(id, filecmd::real);
    }
    registry.register("safari", safari::sim);
    for id in safari::REAL {
        registry.register_standalone(id, safari::real_checks);
    }
    registry.register_real("SAF-IDLE", safari::idle_real);
    registry.register_real("FILE-REPLICA", filerep::real);
    for id in pkginstall::REAL {
        registry.register_standalone(id, pkginstall::real);
    }
    registry.register("readmodels", readmodels::readmodels);
    registry.register("blobs", blobs::blobs);
    registry.register("messages", history::messages);
    registry.register("messages_read", messages_a::messages_read);
    registry.register("messages_truth", history::messages_truth);
    registry.register("contacts", imports::contacts);
    registry.register("notes", imports::notes);
    registry.register("usage", usage::cases);
    registry.register("phone", calls::phone);
    registry.register("usage", imports::usage);
    registry.register("native_media_yield", native::media_yield);
    registry.register("native_media_posters", native::media_posters);
    registry.register("native_media_export_safety", native::media_export_safety);
    registry.register("WEB-REM-PARITY", native::reminders_parity);
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
    registry.register("messages_commands", messages_b::dispatch);
    registry.register("files_native", files_native::files_native);
    registry.register("robustness", robust::dispatch);
    registry.register("command_journey", journey::command_journey);
    registry.register("files_transform", shortcuts::files_transform);
    registry.register("shortcuts_actions", shortcuts::shortcuts_actions);
    registry.register("shortcuts_checks", shortcuts::shortcuts_checks);
    registry.register("shortcuts_real_only", shortcuts::shortcuts_real_only);
    registry.register("mac", mac::mac);
    registry.register("calendar_journey", calendar::calendar_journey);
    registry
}

#[must_use]
pub fn implied_step(id: &str) -> Option<&'static str> {
    if filecmd::SIM.contains(&id) {
        Some("file_commands")
    } else if safari::SIM.contains(&id) {
        Some("safari")
    } else {
        None
    }
}
