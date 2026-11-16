use crate::{Error, Result, localapi::LocalApi};
use serde::Serialize;
use serde_json::{Value, json};
use std::net::IpAddr;

pub const SSH_PORT: u16 = 22;
pub const POLICY_FILE: &str = "https://login.tailscale.com/admin/acls/file";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub os: String,
    pub online: bool,
    pub ssh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Reach {
    pub port: u16,
    pub address: String,
    pub allowed: Vec<Device>,
    pub blocked: Vec<Device>,
}

impl Reach {
    #[must_use]
    pub fn rule(&self) -> Option<Value> {
        (!self.blocked.is_empty()).then(|| {
            json!({
                "action": "accept",
                "src": self.blocked.iter().map(|device| device.address.clone()).collect::<Vec<_>>(),
                "dst": [format!("{}:{}", self.address, self.port)],
            })
        })
    }
}

impl Reach {
    #[must_use]
    pub fn rule_text(&self) -> Option<String> {
        self.rule().map(|rule| {
            format!(
                "{{\"action\": \"accept\", \"src\": {}, \"dst\": {}}}",
                rule["src"], rule["dst"]
            )
        })
    }
}

fn parse_ip(text: &str) -> Option<IpAddr> {
    text.split('/').next()?.parse().ok()
}

fn bits(address: IpAddr) -> (u128, u32) {
    match address {
        IpAddr::V4(v4) => (u128::from(u32::from(v4)), 32),
        IpAddr::V6(v6) => (u128::from(v6), 128),
    }
}

fn same_family(a: IpAddr, b: IpAddr) -> bool {
    a.is_ipv4() == b.is_ipv4()
}

fn matches(pattern: &str, address: IpAddr) -> bool {
    let pattern = pattern.trim();
    if pattern == "*" {
        return true;
    }
    if let Some((first, last)) = pattern.split_once('-') {
        let (Ok(first), Ok(last)) = (first.parse::<IpAddr>(), last.parse::<IpAddr>()) else {
            return false;
        };
        return same_family(first, address)
            && same_family(last, address)
            && bits(first).0 <= bits(address).0
            && bits(address).0 <= bits(last).0;
    }
    let Some(network) = parse_ip(pattern) else {
        return false;
    };
    if !same_family(network, address) {
        return false;
    }
    let (network, width) = bits(network);
    let prefix = pattern
        .split_once('/')
        .and_then(|(_, length)| length.parse::<u32>().ok())
        .unwrap_or(width)
        .min(width);
    if prefix == 0 {
        return true;
    }
    let shift = width - prefix;
    (network >> shift) == (bits(address).0 >> shift)
}

fn tcp(rule: &Value) -> bool {
    rule["IPProto"]
        .as_array()
        .is_none_or(|protocols| protocols.is_empty() || protocols.iter().any(|p| p == 6))
}

fn port_in(range: &Value, port: u16) -> bool {
    let first = range["First"].as_u64().unwrap_or(0);
    let last = range["Last"].as_u64().unwrap_or(0);
    (first..=last).contains(&u64::from(port))
}

#[must_use]
pub fn accepts(rules: &Value, source: &[IpAddr], own: &[IpAddr], port: u16) -> bool {
    rules.as_array().is_some_and(|rules| {
        rules.iter().filter(|rule| tcp(rule)).any(|rule| {
            let from = rule["SrcIPs"].as_array().is_some_and(|patterns| {
                patterns
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|pattern| source.iter().any(|address| matches(pattern, *address)))
            });
            from && rule["DstPorts"].as_array().is_some_and(|destinations| {
                destinations.iter().any(|destination| {
                    port_in(&destination["Ports"], port)
                        && destination["IP"].as_str().is_some_and(|pattern| {
                            own.iter().any(|address| matches(pattern, *address))
                        })
                })
            })
        })
    })
}

fn addresses(value: &Value) -> Vec<IpAddr> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_ip)
                .collect()
        })
        .unwrap_or_default()
}

fn short_name(peer: &Value) -> String {
    peer["DNSName"]
        .as_str()
        .and_then(|name| name.split('.').next())
        .filter(|name| !name.is_empty())
        .or_else(|| peer["HostName"].as_str())
        .unwrap_or("a device")
        .to_owned()
}

#[must_use]
pub fn from_status(status: &Value, rules: &Value, port: u16) -> Reach {
    let own = addresses(&status["Self"]["TailscaleIPs"]);
    let address = own
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| own.first())
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut peers: Vec<&Value> = status["Peer"]
        .as_object()
        .map(|peers| peers.values().collect())
        .unwrap_or_default();
    peers.sort_by_key(|peer| (!peer["Online"].as_bool().unwrap_or(false), short_name(peer)));
    let (mut allowed, mut blocked) = (Vec::new(), Vec::new());
    for peer in peers {
        let source = addresses(&peer["TailscaleIPs"]);
        let Some(first) = source
            .iter()
            .find(|address| address.is_ipv4())
            .or_else(|| source.first())
        else {
            continue;
        };
        let device = Device {
            name: short_name(peer),
            address: first.to_string(),
            os: peer["OS"].as_str().unwrap_or_default().to_owned(),
            online: peer["Online"].as_bool().unwrap_or(false),
            ssh: accepts(rules, &source, &own, SSH_PORT),
        };
        if accepts(rules, &source, &own, port) {
            allowed.push(device);
        } else {
            blocked.push(device);
        }
    }
    Reach {
        port,
        address,
        allowed,
        blocked,
    }
}

async fn document(api: &LocalApi, path: &str) -> Result<Value> {
    Ok(api
        .request(reqwest::Method::GET, path, None, None)
        .await?
        .error_for_status()?
        .json()
        .await?)
}

pub async fn reach(api: &LocalApi, port: u16) -> Result<Reach> {
    let status = document(api, "status").await?;
    if status["BackendState"] != "Running" {
        return Err(Error::Contract("Tailscale is not running"));
    }
    let rules = document(api, "debug-packet-filter-rules").await?;
    Ok(from_status(&status, &rules, port))
}

pub async fn device_name(api: &LocalApi, address: IpAddr) -> Option<String> {
    let found = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        document(api, &format!("whois?addr={address}")),
    )
    .await
    .ok()?
    .ok()?;
    found["Node"]["Name"]
        .as_str()
        .and_then(|name| name.split('.').next())
        .filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(|name| name.chars().take(40).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn status() -> Value {
        json!({
            "BackendState": "Running",
            "Self": {"TailscaleIPs": ["100.101.1.10", "fd7a:115c:a1e0::aa:10"]},
            "Peer": {
                "a": {"DNSName": "air.tail.ts.net.", "OS": "macOS", "Online": true, "TailscaleIPs": ["100.101.1.20"]},
                "b": {"DNSName": "my-vps.tail.ts.net.", "OS": "linux", "Online": false, "TailscaleIPs": ["100.101.1.30"]},
                "c": {"DNSName": "phone.tail.ts.net.", "OS": "iOS", "Online": true, "TailscaleIPs": ["100.101.1.40"]},
            }
        })
    }

    #[test]
    fn only_ssh_from_one_device_blocks_the_route_for_everyone() {
        let rules = json!([{
            "SrcIPs": ["100.101.1.20"],
            "DstPorts": [{"IP": "100.101.1.10", "Ports": {"First": 22, "Last": 22}}],
            "IPProto": [6]
        }]);
        let reach = from_status(&status(), &rules, 8443);
        assert_eq!(reach.allowed, []);
        let names: Vec<_> = reach.blocked.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["air", "phone", "my-vps"]);
        assert!(reach.blocked[0].ssh && !reach.blocked[2].ssh);
        assert_eq!(
            reach.rule_text().unwrap(),
            r#"{"action": "accept", "src": ["100.101.1.20","100.101.1.40","100.101.1.30"], "dst": ["100.101.1.10:8443"]}"#
        );
        assert_eq!(
            reach.rule().unwrap(),
            json!({"action": "accept", "src": ["100.101.1.20", "100.101.1.40", "100.101.1.30"], "dst": ["100.101.1.10:8443"]})
        );
    }

    #[test]
    fn wildcards_ranges_and_default_protocols_allow() {
        let rules = json!([
            {"SrcIPs": ["100.64.0.0/10"], "DstPorts": [{"IP": "*", "Ports": {"First": 0, "Last": 65535}}]},
            {"SrcIPs": ["*"], "DstPorts": [{"IP": "100.101.1.10/32", "Ports": {"First": 8000, "Last": 9000}}], "IPProto": [17]},
        ]);
        let reach = from_status(&status(), &rules, 8443);
        assert_eq!(reach.allowed.len(), 3);
        assert!(reach.rule().is_none());
        let udp_only = json!([{"SrcIPs": ["*"], "DstPorts": [{"IP": "*", "Ports": {"First": 8443, "Last": 8443}}], "IPProto": [17]}]);
        assert_eq!(from_status(&status(), &udp_only, 8443).blocked.len(), 3);
        let range = json!([{"SrcIPs": ["100.101.1.25-100.101.1.35"], "DstPorts": [{"IP": "100.101.1.10", "Ports": {"First": 8443, "Last": 8443}}], "IPProto": [6]}]);
        let reach = from_status(&status(), &range, 8443);
        assert_eq!(reach.allowed.len(), 1);
        assert_eq!(reach.allowed[0].name, "my-vps");
    }

    #[test]
    fn a_rule_for_another_destination_does_not_count() {
        let rules = json!([{"SrcIPs": ["*"], "DstPorts": [{"IP": "100.101.1.20", "Ports": {"First": 8443, "Last": 8443}}]}]);
        assert_eq!(from_status(&status(), &rules, 8443).blocked.len(), 3);
        assert!(!matches("100.101.1.10", ip("fd7a:115c:a1e0::aa:10")));
        assert!(matches("fd7a:115c:a1e0::/48", ip("fd7a:115c:a1e0::aa:10")));
    }
}
