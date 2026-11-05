mod support;

use ab_iphone::delivery::live_payload;
use ab_trust::domain::CommandState;
use serde_json::{Value, json};
use support::{Check, live_row, world};

fn with_tokens(world: &support::World, tasks: &[&str]) -> Check {
    let authority = &world.delivery.authority;
    for task in tasks {
        authority.set_iphone_live_token(&world.phone.id, task, &"a".repeat(64))?;
    }
    Ok(())
}

fn handle(world: &support::World) -> Result<String, Box<dyn std::error::Error>> {
    let raw =
        ab_iphone::delivery::live_handle(&world.phone.encryption_public, "private-task-marker");
    assert_eq!(raw.len(), 32);
    Ok(raw)
}

#[test]
fn start_push_carries_no_private_text() -> Check {
    let world = world()?;
    with_tokens(&world, &["_start"])?;
    let phone = world.delivery.authority.principal(&world.phone.id)?;
    let row = live_row(
        "c1",
        &world.phone.id,
        "iphone.live.start",
        CommandState::Admitted,
    );
    let command: Value = serde_json::from_str(row.command.get())?;
    let (_, aps) = live_payload(&phone, &command, 100)?;
    let text = aps.get();
    assert!(!text.contains("private-title-marker"), "{text}");
    assert!(!text.contains("private-status-marker"), "{text}");
    assert!(!text.contains("private-task-marker"), "{text}");
    let parsed: Value = serde_json::from_str(text)?;
    assert_eq!(parsed["attributes"]["taskID"], handle(&world)?.as_str());
    assert_eq!(parsed["content-state"]["progress"], json!(0));
    Ok(())
}

#[test]
fn update_and_end_pushes_carry_no_private_text_and_use_the_opaque_token() -> Check {
    let world = world()?;
    let opaque = handle(&world)?;
    with_tokens(&world, &[&opaque])?;
    let phone = world.delivery.authority.principal(&world.phone.id)?;
    for op in ["iphone.live.update", "iphone.live.end"] {
        let row = live_row("c1", &world.phone.id, op, CommandState::Admitted);
        let command: Value = serde_json::from_str(row.command.get())?;
        let (token, aps) = live_payload(&phone, &command, 100)?;
        assert_eq!(token, "a".repeat(64));
        assert!(!aps.get().contains("private-"), "{}", aps.get());
    }
    Ok(())
}

#[test]
fn an_activity_started_before_the_upgrade_keeps_its_raw_task_token() -> Check {
    let world = world()?;
    with_tokens(&world, &["private-task-marker"])?;
    let phone = world.delivery.authority.principal(&world.phone.id)?;
    let row = live_row(
        "c1",
        &world.phone.id,
        "iphone.live.update",
        CommandState::Admitted,
    );
    let command: Value = serde_json::from_str(row.command.get())?;
    let (token, aps) = live_payload(&phone, &command, 100)?;
    assert_eq!(token, "a".repeat(64));
    assert!(!aps.get().contains("private-"), "{}", aps.get());
    Ok(())
}

#[test]
fn real_state_reaches_a_new_app_only_through_the_sealed_channel() -> Check {
    let world = world()?;
    with_tokens(&world, &["_start"])?;
    world.rows.0.lock().map_err(|_| "lock")?.push(live_row(
        "c1",
        &world.phone.id,
        "iphone.live.start",
        CommandState::Admitted,
    ));
    let phone = world.delivery.authority.principal(&world.phone.id)?;
    let (wakes, pushes) = world.delivery.packets(&phone, 100, true, true)?;
    assert_eq!(pushes.len(), 1);
    let pushed = pushes[0].packet.live_activity.as_ref().ok_or("aps")?.get();
    assert!(!pushed.contains("private-"), "{pushed}");
    assert_eq!(
        wakes.len(),
        1,
        "the phone is woken so it fetches the sealed state"
    );
    assert!(wakes[0].live_activity.is_none());

    let path = "/v1/iphone/pending";
    let old: support::Check = (|| {
        let request = serde_json::from_slice(&world.phone.body(path, b"")?)?;
        let answer = world.delivery.handle(path, &request, 100)?;
        let packets: Vec<ab_iphone::Packet> = serde_json::from_str(answer.get())?;
        assert!(
            packets.is_empty(),
            "an old app is not sent commands it would reject"
        );
        Ok(())
    })();
    old?;

    let request = serde_json::from_slice(&world.phone.body(path, br#"{"live_state":true}"#)?)?;
    let answer = world.delivery.handle(path, &request, 100)?;
    let packets: Vec<ab_iphone::Packet> = serde_json::from_str(answer.get())?;
    assert_eq!(packets.len(), 1);
    let plain = ab_wire::crypto::open(
        world.phone.encryption_private,
        &world.phone.id,
        &packets[0],
        &world.phone.gateway_key,
    )?;
    let job: Value = serde_json::from_slice(&plain)?;
    assert_eq!(job["command"]["args"]["title"], "private-title-marker");
    assert_eq!(job["command"]["args"]["status"], "private-status-marker");
    assert_eq!(job["command"]["op"], "iphone.live.start");

    let receipt = serde_json::to_vec(&json!({
        "command_id": "c1",
        "request_hash": "hash-c1",
        "state": "observed",
        "data": {"applied": true},
    }))?;
    let path = "/v1/iphone/result";
    let request = serde_json::from_slice(&world.phone.body(path, &receipt)?)?;
    world.delivery.handle(path, &request, 100)?;
    let path = "/v1/iphone/pending";
    let request = serde_json::from_slice(&world.phone.body(path, br#"{"live_state":true}"#)?)?;
    let answer = world.delivery.handle(path, &request, 100)?;
    let packets: Vec<ab_iphone::Packet> = serde_json::from_str(answer.get())?;
    assert!(packets.is_empty(), "a receipted state is not sent again");
    Ok(())
}

#[test]
fn the_shared_vector_matches_the_opaque_handle() -> Check {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let golden: Value = serde_json::from_str(include_str!(
        "../../../../contracts/golden/iphone-pairing.json"
    ))?;
    let key = STANDARD.decode(golden["encryption_key"].as_str().ok_or("key")?)?;
    assert_eq!(
        ab_iphone::delivery::live_handle(&key, golden["live_task"].as_str().ok_or("task")?),
        golden["live_handle"].as_str().ok_or("handle")?
    );
    Ok(())
}
