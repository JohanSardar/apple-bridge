#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    consent::Site,
    helper_real::truth,
    relay_host::{RelayHost, quote, remote},
    remote_real::{host_and_port, public_ip, tls_checks},
    webauthn::{COOKIE, Owner},
};
use lab_probe::{Common, Local, Seen, Watch, new_id, title_observer};
use lab_relay::PairCode;
use lab_runner::{Assertion, Evidence};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::TcpStream,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tungstenite::{Message, client::IntoClientRequest};

pub const SIM_BACKED: [&str; 3] = [
    "WEB-RELAY-CONNECT",
    "WEB-RELAY-COMMAND",
    "WEB-RELAY-PUBLIC-AGENTS",
];
pub const NATIVE: [&str; 3] = [
    "WEB-RELAY-REPLICA",
    "WEB-RELAY-NATIVE-AGENTS",
    "WEB-RELAY-REPLICA-BLOBS",
];
pub const FUNNEL: &str = "WEB-REMOTE-FUNNEL";
const TAILSCALE: &str = "/opt/homebrew/bin/tailscale";
const LIVE_SAMPLES: usize = 200;
const LIST: &str = "Bridge Test";

#[must_use]
pub fn sim_backed(id: &str) -> bool {
    SIM_BACKED.contains(&id) || id == FUNNEL
}
#[must_use]
pub fn handles(id: &str) -> bool {
    sim_backed(id) || NATIVE.contains(&id)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn serve_status() -> Option<Vec<u8>> {
    let output = Command::new(TAILSCALE)
        .args(["serve", "status", "--json"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}
fn mapping_preserved(before: &[u8], during: Option<&[u8]>, host: &str, port: &str) -> bool {
    let original: Value = serde_json::from_slice(before).unwrap_or(Value::Null);
    let mut changed: Value = during
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .unwrap_or(Value::Null);
    let authority = format!("{host}:{port}");
    let mut own_only = during.is_some();
    for key in ["TCP", "Web", "AllowFunnel"] {
        let index = if key == "TCP" {
            port
        } else {
            authority.as_str()
        };
        let Some(section) = changed.get_mut(key).and_then(Value::as_object_mut) else {
            own_only = false;
            continue;
        };
        own_only &= section.contains_key(index);
        section.remove(index);
        let empty = section.is_empty();
        if empty && original[key].is_null() {
            changed.as_object_mut().map(|map| map.remove(key));
        }
    }
    own_only && original == changed
}
fn status_of(world: &World) -> Value {
    std::fs::read(world.gateway_home.join("remote/status.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null)
}
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).max(1);
    sorted.get(rank - 1).copied().unwrap_or_default()
}
fn stats(values: &[f64]) -> (f64, f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    (
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.95),
        sorted.last().copied().unwrap_or_default(),
    )
}
fn local_for(world: &World) -> Result<Local> {
    let address = world.runtime["api_address"]
        .as_str()
        .ok_or_else(|| invalid("gateway API address"))?
        .to_owned();
    let token = world.gateway_cli(&["token", "create", "--scope", "admin"])?;
    Local::new(&Common {
        url: format!("http://{address}"),
        host: address,
        token: token.trim().to_owned(),
        ..Common::default()
    })
    .map_err(|error| invalid(error.to_string()))
}
fn with_fields(mut base: Value, fields: &Value) -> Value {
    if let (Some(target), Some(extra)) = (base.as_object_mut(), fields.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    base
}
fn ssh_ok(script: &str) -> bool {
    remote(script).is_ok_and(|text| !text.is_empty())
}
fn curl_health(url: &str, host: &str, port: &str, ip: &str, seconds: u32) -> bool {
    ssh_ok(&format!(
        "curl --noproxy '*' --fail --silent --show-error --max-time {seconds} --resolve {} {}",
        quote(&format!("{host}:{port}:{ip}")),
        quote(&format!("{url}/v1/health"))
    ))
}
fn ingress_ready(url: &str, host: &str, port: &str, ip: &str, evidence: &mut Evidence) -> bool {
    let started = Instant::now();
    let ready = await_until(120, || curl_health(url, host, port, ip, 1));
    evidence.check(
        "public_ingress_ready_within_120_seconds",
        ready,
        &format!("ready after {} ms", started.elapsed().as_millis()),
    );
    ready
}
fn public_address(host: &str, evidence: &mut Evidence) -> Option<String> {
    let dns = remote(&format!("dig @1.1.1.1 +short A {}", quote(host)));
    let ip = dns.as_ref().ok().and_then(|text| public_ip(text));
    evidence.check("ubuntu_public_dns", dns.is_ok() && ip.is_some(), "");
    ip
}
fn native_ready(world: &World, source: &str) -> bool {
    await_until(30, || {
        let ready = world
            .query("sources.status", &json!({}))
            .is_ok_and(|result| {
                result["records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|record| {
                        record["data"]["source"] == source && record["data"]["state"] == "ready"
                    })
            });
        ready
            && world
                .gateway_home
                .join(format!("helper/session/capture-state/{source}.json"))
                .exists()
    })
}
fn sim_titles(world: &World) -> Vec<String> {
    world
        .control("/inspect", &Value::Null)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .into_iter()
        .flatten()
        .filter(|record| record["kind"] == "upsert")
        .filter_map(|record| record["data"]["title"].as_str().map(str::to_owned))
        .collect()
}
fn effects(world: &World) -> Option<HashMap<String, u64>> {
    let value = world.control("/effects", &Value::Null).ok()?;
    Some(
        value
            .as_object()?
            .iter()
            .filter_map(|(key, count)| Some((key.clone(), count.as_u64()?)))
            .collect(),
    )
}
fn observed_count(world: &World, tag: &str, title: &str) -> Option<usize> {
    let observed = truth(
        world,
        "observe",
        &json!({"list": LIST, "tag": tag, "title": title}),
    )
    .ok()?;
    Some(observed["records"].as_array().map_or(0, Vec::len))
}
fn fixture(world: &World, request: &Value) -> bool {
    truth(world, "fixture", request).is_ok()
}

struct Run<'a> {
    world: &'a mut World,
    evidence: &'a mut Evidence,
    host: RelayHost,
    gateway: Local,
    native: bool,
    id: String,
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if id == FUNNEL {
        return funnel(world, evidence);
    }
    let native = NATIVE.contains(&id.as_str());
    let before = serve_status();
    evidence.check("serve_config_before", before.is_some(), "");
    let Some(before) = before else {
        return Ok(());
    };
    evidence.check("isolated_gateway", true, "");
    let host = RelayHost::open(world)?;
    let gateway = local_for(world)?;
    let mut state = Run {
        world,
        evidence,
        host,
        gateway,
        native,
        id,
    };
    let outcome = state.body(&before);
    state.finish(&before);
    outcome
}

impl Run<'_> {
    fn finish(&mut self, before: &[u8]) {
        let off = self.world.gateway_cli(&["remote", "off"]);
        self.evidence.check("remote_off", off.is_ok(), "");
        self.host.stop();
        let after = serve_status();
        self.evidence.check(
            "serve_config_byte_identical_after_off",
            after.as_deref() == Some(before),
            "",
        );
    }

    fn body(&mut self, before: &[u8]) -> Result<()> {
        if self.native {
            let ready = native_ready(self.world, "reminders");
            self.evidence
                .check("signed_native_helper_reminders_capture_ready", ready, "");
            if !ready {
                return Ok(());
            }
        }
        let on = self
            .world
            .gateway_cli(&["remote", "on", "--via", "system", "--name", "bridge"]);
        self.evidence.check(
            "remote_on_system",
            on.is_ok(),
            &on.as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        if on.is_err() {
            return Ok(());
        }
        let status = status_of(self.world);
        let (host, port) = status["url"]
            .as_str()
            .and_then(host_and_port)
            .ok_or_else(|| invalid("remote URL"))?;
        self.evidence.check(
            "system_backend_port_10000",
            status["backend"] == "system" && port == "10000",
            "",
        );
        let during = serve_status();
        self.evidence.check(
            "existing_handlers_preserved_exactly_while_on",
            mapping_preserved(before, during.as_deref(), &host, &port),
            "",
        );
        let scope = "reminders:write,system:read";
        let Some(code) = self.pair(&status, scope) else {
            return Ok(());
        };
        let Some(ip) = public_address(&host, self.evidence) else {
            return Ok(());
        };
        let url = status["url"].as_str().unwrap_or_default().to_owned();
        if !ingress_ready(&url, &host, &port, &ip, self.evidence) {
            return Ok(());
        }
        tls_checks(&url, &host, &port, &ip, self.evidence);
        let joined = self.join(&code, &ip, scope);
        self.evidence
            .check("ubuntu_joins_and_starts_through_remote", joined.is_ok(), "");
        if joined.is_err() {
            return Ok(());
        }
        let Some(result) = self.snapshot(&status, &code) else {
            return Ok(());
        };
        let enrollment = result["enrollment"].clone();
        self.scenario_checks(&enrollment);
        Ok(())
    }

    fn pair(&mut self, status: &Value, scope: &str) -> Option<PairCode> {
        let encoded = self
            .world
            .gateway_cli(&["pair", "--via-remote", "--scope", scope]);
        self.evidence
            .check("pair_via_remote_cli", encoded.is_ok(), "");
        let encoded = encoded.ok()?;
        for flag in ["--peer-addr", "--dial-replica"] {
            let refused = self
                .world
                .gateway_cli(&["pair", "--via-remote", flag, "127.0.0.1:7443"])
                .is_err();
            self.evidence.check(
                &format!("remote_pair_rejects_{}", flag.trim_start_matches("--")),
                refused,
                "",
            );
        }
        let code = PairCode::decode(encoded.trim()).ok();
        self.evidence.check(
            "remote_code_public_url_and_gateway_pin",
            code.as_ref().is_some_and(|code| {
                code.remote
                    && code.fingerprint.len() == 64
                    && Some(code.address.as_str()) == status["url"].as_str()
            }),
            "",
        );
        let peer_loopback = self.world.runtime["peer_address"]
            .as_str()
            .and_then(|address| address.rsplit_once(':'))
            .is_some_and(|(host, _)| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            });
        self.evidence
            .check("gateway_peer_loopback_only", peer_loopback, "");
        code
    }

    fn join(&mut self, code: &PairCode, ip: &str, scope: &str) -> Result<()> {
        self.host.upload()?;
        let encoded = self
            .world
            .gateway_cli(&["pair", "--via-remote", "--scope", scope])?;
        self.host
            .start_replica(encoded.trim(), &code.address, ip, scope)
    }

    fn snapshot(&mut self, status: &Value, code: &PairCode) -> Option<Value> {
        let collection = "reminders.reminder";
        let mut checks: HashMap<&str, bool> = HashMap::from([("gateway_read", false)]);
        let mut result = Value::Null;
        let converged = await_until(20, || {
            let Ok((records, cursor)) = self.gateway.snapshot(collection) else {
                checks.insert("gateway_read", false);
                return false;
            };
            checks.insert("gateway_read", true);
            let digest = lab_probe::records_digest(&records);
            let Ok(input) = self.host.input() else {
                return false;
            };
            let input = with_fields(input, &json!({"Collection": collection}));
            let Ok(replica) = self.host.probe("query", &input) else {
                checks.insert("replica_read", false);
                return false;
            };
            checks.insert("replica_read", true);
            let count = replica["count"].as_u64().unwrap_or(u64::MAX);
            checks.insert("connected", replica["status"]["state"] == "connected");
            checks.insert(
                "count_equal",
                count == records.len() as u64 && (self.native || count > 0),
            );
            checks.insert("digest_equal", replica["digest"] == digest);
            checks.insert("epoch_equal", replica["cursor"]["epoch"] == cursor["epoch"]);
            checks.insert(
                "gateway_watermark_reached",
                replica["cursor"]["sequence"].as_u64() >= cursor["sequence"].as_u64(),
            );
            result = replica;
            checks.values().all(|ok| *ok)
        });
        self.evidence.check(
            "gateway_snapshot_reference",
            checks.get("gateway_read").copied().unwrap_or(false),
            "",
        );
        let status_ok = converged
            && result["status"]["snapshots"].as_u64().unwrap_or(0) > 0
            && result["status"]["snapshot_pages"].as_u64().unwrap_or(0) > 0;
        let failed: Vec<&&str> = checks
            .iter()
            .filter(|(_, ok)| !**ok)
            .map(|(name, _)| name)
            .collect();
        self.evidence.check(
            "public_relay_snapshot_converges",
            status_ok,
            &format!("unmet: {failed:?}"),
        );
        let enrollment = &result["enrollment"];
        self.evidence.check(
            "public_tls_plus_pinned_gateway_identity",
            enrollment["remote"] == true
                && enrollment["address"] == status["url"]
                && enrollment["fingerprint"] == code.fingerprint.as_str(),
            "",
        );
        self.evidence.check(
            "join_and_feed_forced_through_public_relay",
            result["public_connections"].as_u64().unwrap_or(0) >= 2,
            &format!(
                "records {} pages {} public connections {}",
                result["count"], result["status"]["snapshot_pages"], result["public_connections"]
            ),
        );
        converged.then_some(result)
    }

    fn scenario_checks(&mut self, enrollment: &Value) {
        let id = self.id.clone();
        if id == "WEB-RELAY-COMMAND" {
            self.command_checks();
        }
        if self.native && id == "WEB-RELAY-REPLICA-BLOBS" {
            self.native_blob_checks();
            return;
        }
        if (self.native || id == "WEB-RELAY-PUBLIC-AGENTS") && !enrollment.is_null() {
            self.public_agent_checks(enrollment);
        }
        if self.native && id == "WEB-RELAY-REPLICA" {
            self.postburst_checks();
        }
        if !enrollment.is_null() {
            self.security_checks(enrollment);
        }
    }

    fn command_checks(&mut self) {
        let title = format!("[ab-test {}] forwarded reminder", new_id("relay"));
        let Ok(input) = self.host.input() else {
            return;
        };
        let input = with_fields(input, &json!({"Title": title}));
        let world = &*self.world;
        let outcome = self.host.exchange("command", &input, &mut || {
            if sim_titles(world).contains(&title) {
                Ok(json!({"truth_verified": true}))
            } else {
                Err(invalid("independent create not visible"))
            }
        });
        let good = outcome.as_ref().is_ok_and(|result| {
            [
                "create_observed",
                "create_read_your_write",
                "delete_observed",
                "delete_read_your_write",
            ]
            .iter()
            .all(|name| result[*name] == true)
        });
        self.evidence.check(
            "remote_forwarded_create_delete_and_read_your_write",
            good,
            "",
        );
        if !good {
            return;
        }
        self.evidence.check(
            "independent_forwarded_delete_absence",
            !sim_titles(self.world).contains(&title),
            "",
        );
        let once = effects(self.world)
            .is_some_and(|map| map.len() == 2 && map.values().all(|count| *count == 1));
        self.evidence
            .check("forwarded_commands_execute_once", once, "");
    }

    fn agent_visible(&self, tag: &str, suffix: &str) -> Option<bool> {
        let title = format!("{tag}{suffix}");
        if self.native {
            return observed_count(self.world, tag, &title).map(|count| count == 1);
        }
        Some(sim_titles(self.world).contains(&title))
    }

    fn public_agent_checks(&mut self, enrollment: &Value) {
        let tag = format!("[ab-test {}]", new_id("relay-agent"));
        if self.native {
            let created = fixture(
                self.world,
                &json!({"action": "create_list", "list": LIST, "tag": tag}),
            );
            self.evidence
                .check("native_agent_fixture_list", created, "");
            if !created {
                return;
            }
        }
        self.agents_with_owner(&tag, enrollment);
        if self.native {
            let cleaned = fixture(
                self.world,
                &json!({"action": "cleanup", "list": LIST, "tag": tag}),
            );
            self.evidence
                .check("native_agent_tagged_cleanup", cleaned, "");
        }
    }

    fn agents_with_owner(&mut self, tag: &str, enrollment: &Value) {
        let owner = Site::new(self.world).and_then(|site| {
            let owner = site.owner_session(self.world)?;
            Ok((site, owner))
        });
        self.evidence
            .check("relay_agent_owner_session", owner.is_ok(), "");
        let Ok((site, owner)) = owner else {
            return;
        };
        let socket = live_socket(&site, &owner);
        self.evidence
            .check("relay_agent_owner_live_connection", socket.is_ok(), "");
        let Ok(mut socket) = socket else {
            return;
        };
        let (full_id, approve_id) = (new_id("cmd"), new_id("cmd"));
        let Ok(input) = self.host.input() else {
            return;
        };
        let input = with_fields(
            input,
            &json!({"Prefix": tag, "FullID": full_id, "ApproveID": approve_id}),
        );
        let replica_id = enrollment["replica_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let outcome = {
            let (world, evidence, host) = (&*self.world, &mut *self.evidence, &self.host);
            host.exchange("agents", &input, &mut || {
                approve_through_owner(
                    world,
                    evidence,
                    &mut socket,
                    &owner,
                    tag,
                    &approve_id,
                    &replica_id,
                )
            })
        };
        let good = outcome.as_ref().is_ok_and(|result| {
            [
                "read_denied",
                "full_observed",
                "approve_queued",
                "approved_observed",
                "read_your_write",
                "cleanup_observed",
            ]
            .iter()
            .all(|name| result[*name] == true)
        });
        self.evidence.check(
            "public_replica_agent_levels_and_command_read_your_write",
            good,
            "",
        );
        if !good {
            return;
        }
        let seen = |suffix: &str| self.agent_visible(tag, suffix);
        let gone = [" full", " approve", " read"]
            .iter()
            .all(|suffix| seen(suffix) == Some(false));
        self.evidence
            .check("independent_public_agent_delete_absence", gone, "");
        if !self.native {
            let once = effects(self.world).is_some_and(|map| {
                map.get(&full_id) == Some(&1) && map.get(&approve_id) == Some(&1)
            });
            self.evidence
                .check("public_agent_creates_execute_once", once, "");
        }
    }

    fn security_checks(&mut self, enrollment: &Value) {
        let Ok(input) = self.host.input() else {
            return;
        };
        let replay = self.host.probe("replay", &input);
        self.evidence.check(
            "captured_public_auth_frame_replay_rejected",
            replay.as_ref().is_ok_and(|value| {
                value["capture_authenticated"] == true
                    && value["fresh_nonce"] == true
                    && value["replay_rejected"] == true
            }),
            "",
        );
        if replay.is_err() {
            return;
        }
        let replica_id = enrollment["replica_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let revoked_at = Arc::new(Mutex::new(0_i128));
        let stamp = revoked_at.clone();
        let world = &*self.world;
        let closed = self.host.exchange("revocation", &input, &mut || {
            if let Ok(mut at) = stamp.lock() {
                *at = jiff::Timestamp::now().as_nanosecond();
            }
            world
                .gateway_cli(&["revoke", &replica_id])
                .map(|_| json!({"revoked": true}))
        });
        let closed_at = closed
            .as_ref()
            .ok()
            .and_then(|value| value["closed_gateway_wall_ns"].as_i64())
            .filter(|at| *at > 0);
        self.evidence.check(
            "public_replica_feed_closes_on_revocation",
            closed_at.is_some(),
            "",
        );
        let (Some(closed_at), Ok(closed)) = (closed_at, closed) else {
            return;
        };
        let revoked = revoked_at.lock().map_or(0, |at| *at);
        let uncertainty = closed["clock_uncertainty_ms"].as_f64().unwrap_or_default();
        #[allow(clippy::cast_precision_loss)]
        let elapsed = (i128::from(closed_at) - revoked) as f64 / 1_000_000.0;
        self.evidence.check(
            "relay_revocation_closes_within_one_second",
            elapsed >= -uncertainty && elapsed + uncertainty < 1000.0,
            &format!("closed after {elapsed:.3} ms, clock uncertainty {uncertainty:.3} ms"),
        );
        let reported = await_until(1, || {
            self.host
                .input()
                .map(|input| with_fields(input, &json!({"Collection": "reminders.reminder"})))
                .and_then(|input| {
                    self.host
                        .probe("query", &input)
                        .map_err(|e| invalid(e.to_string()))
                })
                .is_ok_and(|local| local["status"]["state"] != "connected")
        });
        self.evidence.check(
            "running_ubuntu_replica_reports_revoked_connection_closed",
            reported,
            "",
        );
    }

    fn native_blob_checks(&mut self) {
        if !self.blob_stage() {
            return;
        }
        let tag = format!("[ab-test {}]", new_id("relay-blob"));
        let created = fixture(
            self.world,
            &json!({"action": "create_list", "list": LIST, "tag": tag}),
        );
        self.evidence.check("blob_burst_fixture_list", created, "");
        if !created {
            return;
        }
        let burst = truth(
            self.world,
            "fixture",
            &json!({"action": "burst", "list": LIST, "tag": tag, "count": LIVE_SAMPLES}),
        );
        let landed = burst
            .as_ref()
            .ok()
            .and_then(|value| value["effects"].as_array().map(Vec::len))
            .unwrap_or(0);
        self.evidence.check(
            "native_burst_before_timed_blob_stage",
            landed == LIVE_SAMPLES,
            &format!("burst effects {landed}"),
        );
        if landed == LIVE_SAMPLES {
            self.post_burst_blob();
        }
        let cleaned = fixture(
            self.world,
            &json!({"action": "cleanup", "list": LIST, "tag": tag}),
        );
        self.evidence
            .check("blob_burst_tagged_fixture_cleanup", cleaned, "");
    }

    fn blob_stage(&mut self) -> bool {
        let started = Instant::now();
        let staged = self.world.blob_control(&json!({"action": "relay-fixture"}));
        self.evidence.check(
            "synthetic_blob_staged_in_isolated_native_gateway",
            staged.is_ok(),
            &format!("nominal stage {} ms", started.elapsed().as_millis()),
        );
        let Ok(fixture) = staged else {
            return false;
        };
        let Ok(input) = self.host.input() else {
            return false;
        };
        let size = fixture["size"].as_i64().unwrap_or(-1);
        let input = with_fields(
            input,
            &json!({"Title": fixture["title"], "Digest": fixture["digest"], "Size": size}),
        );
        let blob = self.host.probe("blob", &input);
        self.evidence.check(
            "blob_bytes_through_public_relay_match_gateway_digest",
            blob.as_ref().is_ok_and(|blob| {
                blob["digest_matches"] == true && blob["bytes"].as_i64() == Some(size)
            }),
            &format!("relay blob bytes {size}"),
        );
        blob.is_ok()
    }

    fn post_burst_blob(&mut self) {
        let started = Instant::now();
        let outcome = self.world.blob_control(&json!({"action": "relay-fixture"}));
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let message = outcome
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let blocked = message.contains("timed out") || message.contains("500");
        let detail = if blocked {
            format!(
                "LIVE-FIX: native blob fixture failed after {elapsed:.3} ms; default control timeout unchanged"
            )
        } else {
            format!("stage {elapsed:.3} ms")
        };
        self.evidence.assertions.push(Assertion {
            name: "post_burst_blob_stage_available".into(),
            passed: outcome.is_ok(),
            status: blocked.then(|| "blocked".to_owned()),
            detail,
        });
        self.evidence.passed &= outcome.is_ok();
    }

    fn postburst_checks(&mut self) {
        let tag = format!("[ab-test {}]", new_id("relay-postburst"));
        let created = fixture(
            self.world,
            &json!({"action": "create_list", "list": LIST, "tag": tag}),
        );
        self.evidence.check("post_burst_fixture_list", created, "");
        if !created {
            return;
        }
        self.postburst_exchange(&tag);
        let cleaned = fixture(
            self.world,
            &json!({"action": "cleanup", "list": LIST, "tag": tag}),
        );
        self.evidence
            .check("post_burst_tagged_fixture_cleanup", cleaned, "");
    }

    fn postburst_exchange(&mut self, tag: &str) {
        let title = format!("{tag} post-burst delete");
        let delete_id = new_id("cmd");
        let Ok(input) = self.host.input() else {
            return;
        };
        let input = with_fields(input, &json!({"Title": title, "DeleteID": delete_id}));
        let outcome = {
            let (world, evidence, host) = (&*self.world, &mut *self.evidence, &self.host);
            host.exchange("postburst", &input, &mut || {
                let visible = observed_count(world, tag, &title) == Some(1);
                evidence.check("post_burst_fixture_independently_visible", visible, "");
                if !visible {
                    return Err(invalid("post-burst fixture truth failed"));
                }
                Ok(json!({"burst_finished": true}))
            })
        };
        self.evidence.check(
            "post_burst_forwarded_delete_result_measured",
            outcome
                .as_ref()
                .is_ok_and(|value| value["normal_ttl_ms"].as_i64().unwrap_or(0) > 0),
            "",
        );
        let Ok(measured) = outcome else {
            return;
        };
        let ttl = measured["normal_ttl_ms"].as_f64().unwrap_or_default();
        let elapsed = measured["elapsed_ms"].as_f64().unwrap_or(f64::MAX);
        let succeeded =
            measured["observed"] == true && measured["local_absent"] == true && elapsed <= ttl;
        let unavailable = measured["http_status"] == 503 && measured["error_code"] == "unavailable";
        let blocked = !succeeded && unavailable;
        self.evidence.assertions.push(Assertion {
            name: "post_burst_forwarded_delete_within_normal_TTL".into(),
            passed: succeeded,
            status: blocked.then(|| "blocked".to_owned()),
            detail: if blocked {
                format!("LIVE-FIX: reminders.delete HTTP 503 unavailable after {elapsed:.3} ms; normal TTL {ttl} ms; no redispatch (ADR 0014)")
            } else {
                format!("state {} after {elapsed:.3} ms of {ttl} ms", measured["state"])
            },
        });
        self.evidence.passed &= succeeded;
        if succeeded {
            self.evidence.check(
                "post_burst_forwarded_delete_independent_native_absence",
                observed_count(self.world, tag, &title) == Some(0),
                "",
            );
        }
        self.post_burst_blob();
        self.native_live_checks();
    }

    fn native_live_checks(&mut self) {
        let tag = format!("[ab-test {}]", new_id("relay"));
        let ready = truth(self.world, "observe", &json!({"list": LIST, "tag": tag})).is_ok();
        self.evidence.check(
            "native_reminders_truth_ready",
            ready,
            "AppleTruth Reminders access is unverified; no fixture changes made",
        );
        if !ready {
            return;
        }
        let created = fixture(
            self.world,
            &json!({"action": "create_list", "list": LIST, "tag": tag}),
        );
        self.evidence.check("bridge_test_list_ready", created, "");
        if created {
            self.native_command_checks(&tag);
            self.native_burst(&tag);
        }
        let cleaned = fixture(
            self.world,
            &json!({"action": "cleanup", "list": LIST, "tag": tag}),
        );
        self.evidence
            .check("tagged_native_fixture_cleanup", cleaned, "");
    }

    fn native_command_checks(&mut self, tag: &str) {
        let title = format!("{tag} forwarded command");
        let Ok(input) = self.host.input() else {
            return;
        };
        let input = with_fields(input, &json!({"Title": title}));
        let world = &*self.world;
        let outcome = self.host.exchange("command", &input, &mut || {
            if observed_count(world, tag, &title) == Some(1) {
                Ok(json!({"truth_verified": true}))
            } else {
                Err(invalid("forwarded create not independently visible"))
            }
        });
        let good = outcome.as_ref().is_ok_and(|result| {
            [
                "create_observed",
                "create_read_your_write",
                "delete_observed",
                "delete_read_your_write",
            ]
            .iter()
            .all(|name| result[*name] == true)
        });
        self.evidence.check(
            "forwarded_create_and_delete_are_observed_and_read_your_write",
            good,
            "",
        );
        if !good {
            return;
        }
        self.evidence.check(
            "forwarded_delete_independent_native_absence",
            observed_count(self.world, tag, &title) == Some(0),
            "",
        );
        self.blob_stage();
    }

    fn native_burst(&mut self, tag: &str) {
        let scoped =
            self.world
                .gateway_cli(&["token", "create", "--scope", "reminders:read,system:read"]);
        self.evidence
            .check("gateway_observer_scope", scoped.is_ok(), "");
        let Ok(token) = scoped else {
            return;
        };
        let Some((observer, _watch)) = self.gateway_observer(token.trim(), tag) else {
            return;
        };
        let Ok(input) = self.host.input() else {
            return;
        };
        let input = with_fields(input, &json!({"Prefix": tag}));
        let mut timings: Vec<(String, i64)> = Vec::new();
        let world = &*self.world;
        let outcome = self.host.exchange("live", &input, &mut || {
            let burst = truth(
                world,
                "fixture",
                &json!({"action": "burst", "list": LIST, "tag": tag, "count": LIVE_SAMPLES}),
            )?;
            let raw = burst["effects"]
                .as_array()
                .filter(|raw| raw.len() == LIVE_SAMPLES)
                .ok_or_else(|| invalid("native effects missing"))?;
            let mut out = Vec::new();
            for effect in raw {
                let title = effect["title"].as_str().unwrap_or_default();
                #[allow(clippy::cast_possible_truncation)]
                let wall = effect["effect_wall_ns"].as_f64().unwrap_or(0.0) as i64;
                if wall <= 0 || !title.starts_with(tag) {
                    return Err(invalid("native effect timing malformed"));
                }
                out.push(json!({"title": title, "wall_ns": wall}));
                timings.push((title.to_owned(), wall));
            }
            Ok(Value::Array(out))
        });
        let values: Vec<f64> = outcome
            .as_ref()
            .ok()
            .and_then(|live| live["latencies_ms"].as_array())
            .map(|list| list.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        self.evidence.check(
            "native_effects_reach_ubuntu_local_feed",
            outcome.is_ok() && values.len() == LIVE_SAMPLES,
            "",
        );
        let Ok(live) = outcome else {
            return;
        };
        let (mut gateway_latencies, mut relay_latencies) = (Vec::new(), Vec::new());
        for (index, (title, wall)) in timings.iter().enumerate() {
            let seen = observer.lock().ok().and_then(|map| map.get(title).copied());
            let Some(seen) = seen else {
                self.evidence
                    .check("paired_gateway_observations", false, "");
                return;
            };
            #[allow(clippy::cast_precision_loss)]
            let elapsed = (seen - i128::from(*wall)) as f64 / 1_000_000.0;
            gateway_latencies.push(elapsed);
            relay_latencies.push((values[index] - elapsed).max(0.0));
        }
        self.evidence.check(
            "paired_gateway_observations",
            gateway_latencies.len() == LIVE_SAMPLES,
            "",
        );
        let uncertainty = live["clock_uncertainty_ms"].as_f64().unwrap_or_default();
        let (gateway_p50, gateway_p95, _) = stats(&gateway_latencies);
        let (relay_p50, relay_p95, _) = stats(&relay_latencies);
        let (p50, p95, max) = stats(&values);
        self.evidence.check(
            "relay_R_LIVE_gateway_under_one_second",
            gateway_p95 < 1000.0 && gateway_latencies.len() == LIVE_SAMPLES,
            &format!("apple effect to gateway feed p50 {gateway_p50:.1} p95 {gateway_p95:.1} ms"),
        );
        self.evidence.check(
            "relay_R_LIVE_certification",
            p95 + uncertainty < 1000.0 && values.len() == LIVE_SAMPLES,
            &format!(
                "apple effect to ubuntu feed p50 {p50:.1} p95 {p95:.1} max {max:.1} ms; gateway to ubuntu p50 {relay_p50:.1} p95 {relay_p95:.1} ms; clock uncertainty {uncertainty:.3} ms"
            ),
        );
    }

    fn gateway_observer(&mut self, token: &str, tag: &str) -> Option<(Seen, Watch)> {
        let address = self.world.runtime["api_address"].as_str()?.to_owned();
        let local = Local::new(&Common {
            url: format!("http://{address}"),
            host: address,
            token: token.to_owned(),
            ..Common::default()
        })
        .ok()?;
        let head = local
            .op("records.query", &json!({"collection": "", "limit": 1}))
            .ok()?;
        let seen: Seen = Seen::default();
        let watch = local.watch(&head["cursor"], title_observer(&seen, "", "title", tag));
        self.evidence
            .check("gateway_live_observer", watch.is_ok(), "");
        Some((seen, watch.ok()?))
    }
}

fn live_socket(site: &Site, owner: &Owner) -> Result<tungstenite::WebSocket<TcpStream>> {
    let tcp = TcpStream::connect(&site.address)?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut request = format!("ws://{}/web/v1/live", site.host)
        .into_client_request()
        .map_err(|error| invalid(error.to_string()))?;
    let cookie = owner.cookie.clone().unwrap_or_default();
    let headers = request.headers_mut();
    for (name, value) in [
        ("Origin", site.origin.clone()),
        ("Cookie", format!("{COOKIE}={cookie}")),
        ("Sec-WebSocket-Protocol", "web.1".to_owned()),
    ] {
        headers.insert(name, value.parse().map_err(|_| invalid("socket header"))?);
    }
    let (mut socket, _) =
        tungstenite::client::client(request, tcp).map_err(|error| invalid(error.to_string()))?;
    socket
        .send(Message::text(json!({"t": "hello", "v": 1}).to_string()))
        .map_err(|error| invalid(error.to_string()))?;
    let welcome = socket.read().map_err(|error| invalid(error.to_string()))?;
    let frame: Value = serde_json::from_str(welcome.to_text().unwrap_or("{}"))?;
    if frame["t"] != "welcome" {
        return Err(invalid("welcome failed"));
    }
    Ok(socket)
}

fn approve_through_owner(
    world: &World,
    evidence: &mut Evidence,
    socket: &mut tungstenite::WebSocket<TcpStream>,
    owner: &Owner,
    tag: &str,
    approve_id: &str,
    replica_id: &str,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut event = false;
    while Instant::now() < deadline {
        let Ok(message) = socket.read() else { break };
        let frame: Value =
            serde_json::from_str(message.to_text().unwrap_or("{}")).unwrap_or(Value::Null);
        if frame["t"] == "approval" && frame["command"]["command_id"] == approve_id {
            event = true;
            break;
        }
    }
    evidence.check("public_delegated_approval_live_event", event, "");
    if !event {
        return Err(invalid("approval event missing"));
    }
    let listed = owner.request("GET", "/approvals", "application/json", &[])?;
    let pending: Value = serde_json::from_slice(&listed.body).unwrap_or(Value::Null);
    let composite = listed.status == 200
        && pending["approvals"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|row| {
                row["command_id"] == approve_id
                    && row["principal"].as_str().is_some_and(|principal| {
                        principal.starts_with(&format!("{replica_id}/agent:"))
                    })
            });
    evidence.check(
        "public_pending_approval_bound_to_paired_replica_agent",
        composite,
        "",
    );
    let visible = |suffix: &str| -> Option<bool> {
        let title = format!("{tag}{suffix}");
        if world.helper_app.is_some() {
            return observed_count(world, tag, &title).map(|count| count == 1);
        }
        Some(sim_titles(world).contains(&title))
    };
    let read = visible(" read");
    evidence.check(
        "independent_public_read_agent_has_no_effect",
        read == Some(false),
        "",
    );
    let (full, waiting) = (visible(" full"), visible(" approve"));
    evidence.check(
        "independent_full_effect_and_no_effect_before_consent",
        full == Some(true) && waiting == Some(false),
        "",
    );
    if !composite || read != Some(false) || full != Some(true) || waiting != Some(false) {
        return Err(invalid("relay agent independent pre-approval check failed"));
    }
    let decided = owner.decide(approve_id, "approve");
    let ok = decided.as_ref().is_ok_and(|(status, _)| *status == 200);
    evidence.check("gateway_owner_approves_public_replica_agent", ok, "");
    if !ok {
        return Err(invalid("public agent owner approval failed"));
    }
    let effect = await_until(15, || visible(" approve") == Some(true));
    evidence.check("independent_approved_effect", effect, "");
    Ok(json!({"consent_verified": effect}))
}

fn proxy_status(
    address: &str,
    authority: &str,
    origin: &str,
    path: &str,
    forwarded_host: &str,
    source: &str,
) -> u16 {
    let headers = [
        ("Host", authority),
        ("Origin", origin),
        ("X-Forwarded-Host", forwarded_host),
        ("X-Forwarded-Proto", "https"),
        ("X-Forwarded-For", source),
    ];
    crate::rawhttp::request(address, "POST", path, &headers, Some(&b"{}"[..]), 10)
        .map_or(0, |reply| reply.status)
}

fn proxy_checks(world: &World, status: &Value, evidence: &mut Evidence) {
    let lease: Value = std::fs::read(world.gateway_home.join("remote/lease.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null);
    let target = lease["target"].as_str().unwrap_or_default();
    let rest = target.strip_prefix("http://").unwrap_or_default();
    let (address, prefix) = rest
        .split_once('/')
        .map_or((rest, String::new()), |(a, p)| (a, format!("/{p}")));
    let url = status["url"].as_str().unwrap_or_default();
    let authority = url
        .strip_prefix("https://")
        .unwrap_or_default()
        .trim_end_matches('/');
    let request = |path: &str, forwarded: &str, source: &str| {
        proxy_status(address, authority, url, path, forwarded, source)
    };
    let login = "/auth/login/options";
    evidence.check(
        "direct_loopback_forwarding_spoof_rejected",
        request(login, authority, "192.0.2.40") == 403,
        "",
    );
    evidence.check(
        "proxy_unconfigured_forwarded_host_rejected",
        request(
            &format!("{prefix}{login}"),
            "attacker.invalid",
            "192.0.2.40",
        ) == 403,
        "",
    );
    let own = (0..10).all(|_| request(&format!("{prefix}{login}"), authority, "192.0.2.40") == 200);
    evidence.check(
        "system_original_client_ip_rate_bucket",
        own && request(&format!("{prefix}{login}"), authority, "192.0.2.40") == 429,
        "",
    );
    evidence.check(
        "system_distinct_client_ip_not_loopback_bucket",
        request(&format!("{prefix}{login}"), authority, "192.0.2.41") == 200,
        "",
    );
}

fn funnel_probe(
    host: &RelayHost,
    url: &str,
    ip: &str,
    remote_token: &str,
    local_token: &str,
    off: bool,
    evidence: &mut Evidence,
) {
    let input =
        json!({"URL": url, "IP": ip, "Remote": remote_token, "Local": local_token, "Off": off});
    let result = host.probe("remote", &input);
    evidence.check(&format!("ubuntu_probe_off_{off}"), result.is_ok(), "");
    let Ok(result) = result else {
        return;
    };
    let Some(checks) = result["checks"].as_object() else {
        evidence.check("probe_json", false, "");
        return;
    };
    for (name, passed) in checks {
        evidence.check(name, passed.as_bool() == Some(true), "");
    }
}

fn funnel(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let before = serve_status();
    evidence.check("serve_config_before", before.is_some(), "");
    let Some(before) = before else {
        return Ok(());
    };
    evidence.check("isolated_gateway", true, "");
    let outcome = funnel_body(world, evidence, &before);
    let _ = world.gateway_cli(&["remote", "off"]);
    outcome
}

fn funnel_body(world: &mut World, evidence: &mut Evidence, before: &[u8]) -> Result<()> {
    let on = world.gateway_cli(&["remote", "on", "--via", "tailscale-app", "--name", "bridge"]);
    evidence.check(
        "remote_on_system",
        on.is_ok(),
        &on.as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    if on.is_err() {
        return Ok(());
    }
    let status = status_of(world);
    let (host, port) = status["url"]
        .as_str()
        .and_then(host_and_port)
        .ok_or_else(|| invalid("remote URL"))?;
    evidence.check(
        "system_backend_port_10000",
        status["backend"] == "system" && port == "10000",
        "",
    );
    let cli: Value = world
        .gateway_cli(&["remote", "status"])
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    evidence.check(
        "remote_status_cli_state_backend_url",
        cli["state"] == "on" && cli["backend"] == "system" && cli["url"] == status["url"],
        "",
    );
    proxy_checks(world, &status, evidence);
    let during = serve_status();
    evidence.check(
        "existing_handlers_preserved_exactly_while_on",
        mapping_preserved(before, during.as_deref(), &host, &port),
        "",
    );
    let Some(ip) = public_address(&host, evidence) else {
        return Ok(());
    };
    let url = status["url"].as_str().unwrap_or_default().to_owned();
    tls_checks(&url, &host, &port, &ip, evidence);
    let relay = RelayHost::open(world)?;
    let uploaded = relay.upload_probe();
    evidence.check("probe_copied_to_ubuntu", uploaded.is_ok(), "");
    if uploaded.is_err() {
        return Ok(());
    }
    let remote_token = world.gateway_cli(&["token", "create", "--remote", "--scope", "admin"]);
    evidence.check("remote_token_cli", remote_token.is_ok(), "");
    let local_token = world.gateway_cli(&["token", "create", "--scope", "admin"]);
    let (Ok(remote_token), Ok(local_token)) = (remote_token, local_token) else {
        return Ok(());
    };
    let (remote_token, local_token) = (
        remote_token.trim().to_owned(),
        local_token.trim().to_owned(),
    );
    funnel_probe(
        &relay,
        &url,
        &ip,
        &remote_token,
        &local_token,
        false,
        evidence,
    );
    let restarted = world.restart_gateway();
    evidence.check("gateway_restart", restarted.is_ok(), "");
    let restored = restarted.is_ok()
        && await_until(120, || {
            let now = status_of(world);
            now["state"] == "on" && now["backend"] == "system" && now["url"] == status["url"]
        });
    evidence.check("remote_backend_url_restored", restored, "");
    funnel_probe(
        &relay,
        &url,
        &ip,
        &remote_token,
        &local_token,
        false,
        evidence,
    );
    let started = Instant::now();
    let off = world.gateway_cli(&["remote", "off"]);
    evidence.check("remote_off", off.is_ok(), "");
    funnel_probe(
        &relay,
        &url,
        &ip,
        &remote_token,
        &local_token,
        true,
        evidence,
    );
    let elapsed = started.elapsed();
    evidence.check(
        "off_within_one_second",
        elapsed <= Duration::from_secs(1),
        &format!(
            "remote off to public unreachable {} ms",
            elapsed.as_millis()
        ),
    );
    let after = serve_status();
    evidence.check(
        "serve_config_byte_identical_after_off",
        after.as_deref() == Some(before),
        "",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{handles, mapping_preserved, percentile, sim_backed, stats};

    #[test]
    fn knows_which_scenarios_need_a_simulated_gateway() {
        assert!(sim_backed("WEB-RELAY-COMMAND"));
        assert!(sim_backed("WEB-REMOTE-FUNNEL"));
        assert!(!sim_backed("WEB-RELAY-REPLICA"));
        assert!(handles("WEB-RELAY-REPLICA-BLOBS"));
        assert!(!handles("WEB-RELAY-AUTH"));
    }

    #[test]
    fn mapping_check_ignores_only_the_added_handlers() {
        let before = br#"{"TCP":{"443":{"HTTPS":true}},"Web":{"keep.example:443":{}}}"#;
        let during = br#"{"TCP":{"443":{"HTTPS":true},"10000":{"HTTPS":true}},"Web":{"keep.example:443":{},"me.example:10000":{}},"AllowFunnel":{"me.example:10000":true}}"#;
        assert!(mapping_preserved(
            before,
            Some(during),
            "me.example",
            "10000"
        ));
        let tampered = br#"{"TCP":{"443":{"HTTPS":false},"10000":{}},"Web":{"keep.example:443":{},"me.example:10000":{}},"AllowFunnel":{"me.example:10000":true}}"#;
        assert!(!mapping_preserved(
            before,
            Some(tampered),
            "me.example",
            "10000"
        ));
        assert!(!mapping_preserved(before, None, "me.example", "10000"));
        let missing = br#"{"TCP":{"443":{"HTTPS":true}},"Web":{"keep.example:443":{}}}"#;
        assert!(!mapping_preserved(
            before,
            Some(missing),
            "me.example",
            "10000"
        ));
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let values: Vec<f64> = (1..=200).map(f64::from).collect();
        assert!((percentile(&values, 0.5) - 100.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 0.95) - 190.0).abs() < f64::EPSILON);
        let (p50, p95, max) = stats(&[3.0, 1.0, 2.0]);
        assert!((p50 - 2.0).abs() < f64::EPSILON);
        assert!((p95 - 3.0).abs() < f64::EPSILON);
        assert!((max - 3.0).abs() < f64::EPSILON);
    }
}
