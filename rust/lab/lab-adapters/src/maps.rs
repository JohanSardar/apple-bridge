use crate::{
    assertions::await_until,
    http::{Client, random_id},
    poster::Image,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub const SIM: [&str; 2] = ["MAP-SIM", "LOC-SIM"];
pub const REAL: [&str; 7] = [
    "LOC-IDLE",
    "MAP-IDLE",
    "LOC-CURRENT-REAL",
    "LOC-GEOCODE-REAL",
    "MAP-DIRECTIONS-REAL",
    "MAP-SEARCH-REAL",
    "MAP-SNAPSHOT-REAL",
];
const READ: &str = "location:read,maps:read,system:read";
const WRITE: &str = "maps:write,maps:read";
const FERRY: (f64, f64) = (37.7955, -122.3937);
const BETWEEN: [f64; 2] = [0.0132, 0.0161];

struct Reply {
    ok: bool,
    code: String,
    result: Value,
    blobs: Vec<Value>,
}

fn query(client: &Client, op: &str, args: &Value, deadline: i64) -> Result<Reply> {
    let body = json!({"id": random_id("qry")?, "args": args, "deadline_ms": deadline});
    let (status, value) = client
        .call_raw(op, &body, 70)?
        .ok_or_else(|| Error::Invalid("no query response".into()))?;
    Ok(Reply {
        ok: status == 200 && value["error"].is_null(),
        code: value["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        result: value["result_json"].clone(),
        blobs: value["blobs"].as_array().cloned().unwrap_or_default(),
    })
}

fn open(client: &Client, args: &Value) -> Result<(bool, String)> {
    let body =
        json!({"command_id": random_id("cmd")?, "op": "maps.open", "args": args, "wait_ms": 15000});
    let (status, value) = client
        .call_raw("maps.open", &body, 40)?
        .ok_or_else(|| Error::Invalid("no command response".into()))?;
    Ok((
        status == 200 && value["error"].is_null() && value["state"] == "observed",
        value["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    ))
}

fn meters(lat: f64, lon: f64) -> f64 {
    let dy = (lat - FERRY.0) * 111_320.0;
    let dx = (lon - FERRY.1) * 111_320.0 * FERRY.0.to_radians().cos();
    dx.hypot(dy)
}

fn point(lat: f64, lon: f64) -> Value {
    json!({"lat": lat, "lon": lon})
}

fn route(mode: &str) -> Value {
    json!({"from": point(37.7955, -122.3937), "to": point(37.8087, -122.4098), "mode": mode})
}

fn route_bounds() -> (f64, f64) {
    let straight =
        (BETWEEN[0] * 111_320.0).hypot(BETWEEN[1] * 111_320.0 * 37.8021_f64.to_radians().cos());
    (straight * 1.2 * 0.7, straight * 3.0 * 1.3)
}

fn first_route(reply: &Reply) -> (f64, f64, usize, usize) {
    let route = &reply.result["routes"][0];
    (
        route["distance"].as_f64().unwrap_or(-1.0),
        route["duration"].as_f64().unwrap_or(-1.0),
        route["steps"].as_array().map_or(0, Vec::len),
        route["polyline"].as_array().map_or(0, Vec::len),
    )
}

fn plausible(distance: f64, duration: f64) -> bool {
    let (low, high) = route_bounds();
    distance >= low && distance <= high && duration > 10_000.0 && duration < 36_000_000.0
}

fn code(error: &Reply, expected: &str) -> bool {
    !error.ok && error.code == expected
}

fn scenario_op(id: &str) -> (&'static str, Value) {
    let near = json!({"query": "coffee", "near": point(FERRY.0, FERRY.1), "radius": 5000});
    if id.starts_with("LOC") {
        return (
            "location.geocode",
            json!({"address": "1 Ferry Building, San Francisco, CA 94111"}),
        );
    }
    if id.contains("DIRECTIONS") {
        return ("maps.directions", route("walking"));
    }
    ("maps.search", near)
}

fn sim_extras(world: &World, client: &Client, evidence: &mut Evidence) -> Result<bool> {
    let id = evidence.scenario.clone();
    if id == "MAP-SIM" {
        let snapshot = query(
            client,
            "maps.snapshot",
            &json!({"center": point(FERRY.0, FERRY.1), "span": 0.02, "width": 512, "height": 512}),
            10_000,
        )?;
        evidence.check(
            "snapshot_schema",
            snapshot.ok
                && snapshot.blobs.len() == 1
                && snapshot.blobs[0]["size"].as_u64().unwrap_or(0) > 0,
            "",
        );
        let writer = Client::new(world, WRITE)?;
        let (opened, _) = open(&writer, &point(FERRY.0, FERRY.1))?;
        evidence.check("maps_open_observed", opened, "");
        let (routed, _) = open(&writer, &route("walking"))?;
        evidence.check("maps_route_open_observed", routed, "");
        let incomplete = json!({"from": point(FERRY.0, FERRY.1), "mode": "walking"});
        let (accepted, failure) = open(&writer, &incomplete)?;
        evidence.check(
            "incomplete_route_rejected",
            !accepted && failure == "invalid",
            "",
        );
    }
    if id == "LOC-SIM" {
        let current = query(client, "location.current", &json!({}), 60_000)?;
        let fix = &current.result;
        let stamped = fix["timestamp"]
            .as_str()
            .and_then(|text| text.parse::<jiff::Timestamp>().ok());
        let valid = current.ok
            && fix["lat"].as_f64().is_some_and(|v| v.abs() <= 90.0)
            && fix["lon"].as_f64().is_some_and(|v| v.abs() <= 180.0)
            && fix["accuracy"].as_f64().is_some_and(|v| v >= 0.0)
            && fix["source"] == "mac";
        evidence.check("authorized_fixture_current_schema", valid, "");
        let fresh = stamped.is_some_and(|at| {
            let age = jiff::Timestamp::now().as_millisecond() - at.as_millisecond();
            (0..120_000).contains(&age)
        });
        evidence.check("authorized_fixture_current_is_fresh", fresh, "");
        let invalid = query(
            client,
            "location.current",
            &json!({"unexpected": true}),
            60_000,
        )?;
        evidence.check(
            "current_unknown_fields_rejected",
            code(&invalid, "invalid"),
            "",
        );
    }
    if id == "MAP-SIM" {
        for mode in ["driving", "walking", "transit", "cycling"] {
            let reply = query(client, "maps.directions", &route(mode), 60_000)?;
            let passed = if mode == "transit" {
                code(&reply, "unsupported")
            } else {
                reply.ok
            };
            evidence.check(&format!("transport_mode_{mode}"), passed, "");
        }
        let eta = query(client, "maps.eta", &route("walking"), 60_000)?;
        evidence.check("eta_completed", eta.ok, "");
        let nearby = query(
            client,
            "maps.nearby",
            &json!({"category": "coffee", "near": point(FERRY.0, FERRY.1), "radius": 5000}),
            60_000,
        )?;
        evidence.check("nearby_completed", nearby.ok, "");
    }
    Ok(true)
}

fn generic(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let location = id.starts_with("LOC");
    let (op, args) = scenario_op(&id);
    let mut malformed = args.clone();
    malformed["unexpected"] = json!(true);
    let rejected = query(client, op, &malformed, 60_000)?;
    evidence.check("unknown_fields_rejected", code(&rejected, "invalid"), "");
    let reply = query(client, op, &args, 60_000)?;
    if !reply.ok {
        eprintln!("maps query failure: {op} {}", reply.code);
    }
    evidence.check("query_completed", reply.ok, "");
    if op.contains("directions") {
        let (distance, duration, steps, polyline) = first_route(&reply);
        evidence.check(
            "route_shape",
            reply.result["routes"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
                && plausible(distance, duration)
                && steps > 0
                && polyline > 1,
            "",
        );
        let driving = query(client, "maps.directions", &route("driving"), 60_000)?;
        let (distance, duration, _, _) = first_route(&driving);
        evidence.check(
            "driving_route",
            driving.ok && plausible(distance, duration),
            "",
        );
        for mode in ["walking", "transit", "cycling"] {
            let eta = query(client, "maps.eta", &route(mode), 60_000)?;
            let ok = eta.ok
                && plausible(
                    eta.result["distance"].as_f64().unwrap_or(-1.0),
                    eta.result["duration"].as_f64().unwrap_or(-1.0),
                );
            evidence.check(
                &format!("eta_{mode}_distance_and_millisecond_duration"),
                ok,
                "",
            );
        }
        let cycling = query(client, "maps.directions", &route("cycling"), 60_000)?;
        let (distance, duration, _, polyline) = first_route(&cycling);
        evidence.check(
            "cycling_route",
            cycling.ok && plausible(distance, duration) && polyline > 1,
            "",
        );
        return Ok(());
    }
    let places = reply.result["places"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    evidence.check("public_places_present", !places.is_empty(), "");
    if !location {
        let within = !places.is_empty()
            && places.iter().all(|place| {
                !place["name"].as_str().unwrap_or_default().is_empty()
                    && meters(
                        place["lat"].as_f64().unwrap_or(1000.0),
                        place["lon"].as_f64().unwrap_or(1000.0),
                    ) <= 5000.0
            });
        evidence.check("places_within_radius", within, "");
    }
    if location && let Some(first) = places.first() {
        let error = meters(
            first["lat"].as_f64().unwrap_or(1000.0),
            first["lon"].as_f64().unwrap_or(1000.0),
        );
        evidence.check("known_public_coordinate", error <= 1000.0, "");
        evidence.check(
            "formatted_address_present",
            !first["address"].as_str().unwrap_or_default().is_empty(),
            "",
        );
        let reverse = query(
            client,
            "location.reverse_geocode",
            &json!({"lat": FERRY.0, "lon": FERRY.1}),
            60_000,
        )?;
        let first = reverse.result["places"]
            .get(0)
            .cloned()
            .unwrap_or(Value::Null);
        let valid = reverse.ok
            && !first["address"].as_str().unwrap_or_default().is_empty()
            && meters(
                first["lat"].as_f64().unwrap_or(1000.0),
                first["lon"].as_f64().unwrap_or(1000.0),
            ) <= 1000.0;
        evidence.check("reverse_round_trip", valid, "");
    }
    Ok(())
}

pub fn sim(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_started", true, "");
    let client = Client::new(world, READ);
    evidence.check("scopes_granted", client.is_ok(), "");
    let client = client?;
    sim_extras(world, &client, evidence)?;
    generic(&client, evidence)
}

fn helper_ready(client: &Client) -> bool {
    await_until(20, || {
        query(client, "helper.version", &json!({}), 1000).is_ok_and(|reply| reply.ok)
    })
}

fn cpu_seconds(pid: &str) -> Option<f64> {
    let output = Command::new("ps")
        .args(["-p", pid, "-o", "time="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let fields: Vec<&str> = text.split(':').collect();
    if !(2..=3).contains(&fields.len()) {
        return None;
    }
    let mut total = 0.0;
    for field in fields {
        let value: f64 = field.parse().ok().filter(|v: &f64| *v >= 0.0)?;
        total = total * 60.0 + value;
    }
    Some(total)
}

fn idle_ready(world: &World, evidence: &mut Evidence) -> bool {
    let Some(app) = world.helper_app.as_ref() else {
        return false;
    };
    let Ok(home) = std::env::var("HOME") else {
        return false;
    };
    let report = std::path::Path::new(&home)
        .join("Library/Application Support/AppleBridgeDev")
        .join(format!("{}-idle-permissions.json", evidence.scenario));
    let status = Command::new("open")
        .args(["-n", "-W", "-g", "-a"])
        .arg(app)
        .args(["--args", "status", "--report"])
        .arg(&report)
        .status();
    let permissions: Option<Value> = fs::read(&report)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok());
    let _ = fs::remove_file(&report);
    let (Ok(status), Some(permissions)) = (status, permissions) else {
        return false;
    };
    if !status.success() {
        return false;
    }
    if permissions["full_disk_access"] != "granted" {
        evidence.unverified(
            "full_disk_access_granted",
            "needs Full Disk Access for the signed Helper; capture cannot settle, so no CPU measurement was taken",
        );
        return false;
    }
    await_until(25 * 60, || {
        fs::read(world.gateway_home.join("helper/messages-idle.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .is_some_and(|state| state["idle"] == true)
    })
}

fn idle(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    if evidence.scenario == "MAP-IDLE" && !idle_ready(world, evidence) {
        evidence.check("messages_capture_and_media_settled", false, "");
        return Ok(());
    }
    let Some(app) = world.helper_app.as_ref() else {
        return Err(Error::Invalid("no Helper app".into()));
    };
    let executable = app.join("Contents/MacOS/AppleBridgeHelper");
    let pids = Command::new("pgrep")
        .arg("-f")
        .arg(executable.to_string_lossy().as_ref())
        .output();
    let listed = pids
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let ids: Vec<String> = listed
        .as_ref()
        .map(|text| text.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let single = pids.is_ok_and(|o| o.status.success()) && ids.len() == 1;
    evidence.check("private_helper_pid", single, "");
    if !single {
        return Ok(());
    }
    let pid = ids[0].clone();
    let warmup = Instant::now();
    let mut quiet = 0;
    let mut previous = cpu_seconds(&pid);
    while quiet < 2 && warmup.elapsed() < Duration::from_secs(300) {
        let window = Instant::now();
        thread::sleep(Duration::from_secs(10));
        let next = cpu_seconds(&pid);
        quiet = match (previous, next) {
            (Some(before), Some(after))
                if after >= before
                    && (after - before) / window.elapsed().as_secs_f64() * 100.0 < 2.0 =>
            {
                quiet + 1
            }
            _ => 0,
        };
        previous = next;
    }
    evidence.check("helper_reached_idle", quiet == 2, "");
    let before = cpu_seconds(&pid);
    let started = Instant::now();
    thread::sleep(Duration::from_secs(300));
    let after = cpu_seconds(&pid);
    let seconds = started.elapsed().as_secs_f64();
    let percent = match (before, after) {
        (Some(b), Some(a)) => (a - b) / seconds * 100.0,
        _ => f64::MAX,
    };
    evidence.check(
        "five_minute_idle_cpu_under_2_percent",
        before.is_some() && after >= before && seconds >= 300.0 && percent < 2.0,
        &format!("idle_cpu_percent={percent:.3} idle_seconds={seconds:.0}"),
    );
    let alive = query(client, "helper.version", &json!({}), 1000)?;
    evidence.check("idle_helper_responsive", alive.ok, "");
    Ok(())
}

fn current(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let reply = query(client, "location.current", &json!({}), 60_000)?;
    let fix = &reply.result;
    let age = fix["timestamp"]
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map(|at| jiff::Timestamp::now().as_millisecond() - at.as_millisecond());
    let ok = reply.ok
        && fix["accuracy"].as_f64().is_some_and(|v| v >= 0.0)
        && age.is_some_and(|age| age < 120_000 && age > -1000)
        && fix["source"] == "mac";
    evidence.check("fresh_mac_fix", ok, "");
    Ok(())
}

fn snapshot(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let wide =
        json!({"center": point(37.7955, -122.3937), "span": 0.02, "width": 2048, "height": 512});
    let oversized = query(client, "maps.snapshot", &wide, 10_000)?;
    evidence.check(
        "oversized_snapshot_rejected",
        code(&oversized, "invalid"),
        "",
    );
    let marked = json!({"center": point(37.7955, -122.3937), "span": 0.02, "width": 512, "height": 512, "markers": [point(37.8005, -122.3937)]});
    let id = random_id("qry")?;
    let body = json!({"id": id, "args": marked, "deadline_ms": 60_000});
    let (status, value) = client
        .call_raw("maps.snapshot", &body, 70)?
        .ok_or_else(|| Error::Invalid("no snapshot response".into()))?;
    let blobs = value["blobs"].as_array().cloned().unwrap_or_default();
    let produced = status == 200 && value["error"].is_null() && blobs.len() == 1;
    evidence.check("snapshot_blob", produced, "");
    if !produced {
        return Ok(());
    }
    let digest = blobs[0]["digest"].as_str().unwrap_or_default();
    let fetched = client.request(
        "GET",
        &format!("/v1/blobs/{digest}?record={id}"),
        &[],
        None,
        30,
    );
    evidence.check("blob_read", fetched.is_ok(), "");
    let Ok(fetched) = fetched else {
        return Ok(());
    };
    let image = Image::decode(&fetched.body);
    let sized = image
        .as_ref()
        .is_some_and(|i| i.width == 512 && i.height == 512);
    evidence.check("png_dimensions", sized, "");
    let mut hits = 0;
    if let Some(image) = image.as_ref().filter(|_| sized) {
        for x in 248..264 {
            for y in 60..220 {
                let [r, g, b] = image.rgb16(x, y);
                if r > 50000.0 && g < 30000.0 && b < 30000.0 && image.alpha16(x, y) > 50000.0 {
                    hits += 1;
                }
            }
        }
    }
    evidence.check("northern_marker_appears_north", hits >= 10, "");
    let mut count = 1;
    while count < 17 {
        let repeated = query(client, "maps.snapshot", &marked, 60_000)?;
        if !repeated.ok || repeated.blobs.len() != 1 {
            break;
        }
        count += 1;
    }
    evidence.check("completed_snapshots_release_helper_bytes", count == 17, "");
    Ok(())
}

pub fn real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_started", true, "");
    let client = Client::new(world, READ);
    evidence.check("scopes_granted", client.is_ok(), "");
    let client = client?;
    let ready = helper_ready(&client);
    evidence.check("helper_ready", ready, "");
    if !ready {
        return Ok(());
    }
    match evidence.scenario.as_str() {
        "LOC-IDLE" | "MAP-IDLE" => idle(world, &client, evidence),
        "LOC-CURRENT-REAL" => current(&client, evidence),
        "MAP-SNAPSHOT-REAL" => snapshot(&client, evidence),
        _ => generic(&client, evidence),
    }
}
