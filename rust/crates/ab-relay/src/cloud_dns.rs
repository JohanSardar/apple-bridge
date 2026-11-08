use crate::{Error, Result, hello::valid_host, http::DnsProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    ttl: i64,
    rrdatas: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct Records {
    #[serde(default)]
    rrsets: Vec<Record>,
}
#[derive(Debug, Deserialize)]
struct Token {
    access_token: String,
    expires_in: i64,
}
#[derive(Debug)]
pub struct CloudDns {
    client: reqwest::Client,
    metadata: reqwest::Client,
    base: String,
    suffix: String,
    gate: Mutex<()>,
}
impl CloudDns {
    pub fn new(project: &str, zone: &str, suffix: &str) -> Result<Self> {
        if ![project, zone].iter().all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        }) {
            return Err(Error::Configuration);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Configuration)?;
        let metadata = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            client,
            metadata,
            base: format!(
                "https://dns.googleapis.com/dns/v1/projects/{project}/managedZones/{zone}"
            ),
            suffix: suffix.to_owned(),
            gate: Mutex::new(()),
        })
    }
    async fn records(&self, token: &str, name: &str, kind: &str) -> Result<Vec<Record>> {
        let response = self
            .client
            .get(format!("{}/rrsets", self.base))
            .bearer_auth(token)
            .query(&[("name", name), ("type", kind)])
            .send()
            .await
            .map_err(|_| Error::Denied)?;
        if response.status() != 200 {
            return Err(Error::Denied);
        }
        let raw = crate::client::limited(response, 65536).await?;
        if raw.len() > 65536 {
            return Err(Error::Denied);
        }
        Ok(serde_json::from_slice::<Records>(&raw)?.rrsets)
    }
    async fn propagate(&self, name: &str, value: &str) -> Result<()> {
        if !value.is_empty() {
            for ip in [
                std::net::Ipv4Addr::new(1, 1, 1, 1),
                std::net::Ipv4Addr::new(8, 8, 8, 8),
            ] {
                let config = hickory_resolver::config::ResolverConfig::from_parts(
                    None,
                    vec![],
                    vec![hickory_resolver::config::NameServerConfig::udp(ip.into())],
                );
                let resolver = propagation_resolver(config)?;
                loop {
                    if propagation_visible(&resolver, name, value).await {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
        Ok(())
    }
    async fn publish(&self, name: &str, value: &str) -> Result<()> {
        if value.len() > 128
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(Error::Denied);
        }
        let host = name.strip_prefix("_acme-challenge.").ok_or(Error::Denied)?;
        if !valid_host(host)
            || !(host == self.suffix || host.ends_with(&format!(".{}", self.suffix)))
        {
            return Err(Error::Denied);
        }
        let _guard = self.gate.lock().await;
        let response = self.metadata.get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token").header("Metadata-Flavor","Google").send().await.map_err(|_| Error::Denied)?;
        if response.status() != 200 {
            return Err(Error::Denied);
        }
        let raw = crate::client::limited(response, 8192).await?;
        if raw.len() > 8192 {
            return Err(Error::Denied);
        }
        let token: Token = serde_json::from_slice(&raw)?;
        if token.access_token.is_empty() || token.expires_in <= 0 {
            return Err(Error::Denied);
        }
        let name = format!("{}.", name.trim_end_matches('.'));
        let old = self.records(&token.access_token, &name, "TXT").await?;
        let mut change = serde_json::Map::<String, Value>::new();
        if !old.is_empty() {
            change.insert("deletions".into(), serde_json::to_value(old)?);
        }
        if !value.is_empty() {
            let mut additions = vec![Record {
                name: name.clone(),
                kind: "TXT".into(),
                ttl: 30,
                rrdatas: vec![format!("\"{value}\"")],
            }];
            let hostname = name.trim_start_matches("_acme-challenge.");
            if self
                .records(&token.access_token, hostname, "A")
                .await?
                .is_empty()
            {
                let mut wildcard = self
                    .records(&token.access_token, &format!("*.{}.", self.suffix), "A")
                    .await?;
                let mut record = wildcard.drain(..).next().ok_or(Error::Denied)?;
                hostname.clone_into(&mut record.name);
                additions.push(record);
            }
            change.insert("additions".into(), json!(additions));
        }
        if change.is_empty() {
            return Ok(());
        }
        let response = self
            .client
            .post(format!("{}/changes", self.base))
            .bearer_auth(&token.access_token)
            .json(&change)
            .send()
            .await
            .map_err(|_| Error::Denied)?;
        if response.status() != 200 {
            return Err(Error::Denied);
        }
        self.propagate(&name, value).await?;
        Ok(())
    }
}
async fn propagation_visible(
    resolver: &hickory_resolver::TokioResolver,
    name: &str,
    value: &str,
) -> bool {
    resolver.txt_lookup(name).await.is_ok_and(|answer| {
        answer
            .answers()
            .iter()
            .any(|txt| published(&txt.data, value))
    })
}
fn propagation_resolver(
    config: hickory_resolver::config::ResolverConfig,
) -> Result<hickory_resolver::TokioResolver> {
    let mut builder = hickory_resolver::TokioResolver::builder_with_config(
        config,
        hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
    );
    builder.options_mut().cache_size = 0;
    builder.build().map_err(|_| Error::Configuration)
}
fn published(record: &hickory_resolver::proto::rr::RData, value: &str) -> bool {
    match record {
        hickory_resolver::proto::rr::RData::TXT(txt) => {
            txt.txt_data.iter().flatten().copied().eq(value.bytes())
        }
        _ => false,
    }
}
impl DnsProvider for CloudDns {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(350), self.publish(name, value))
                .await
                .map_err(|_| Error::Denied)?
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::published;
    use hickory_resolver::proto::rr::{RData, rdata::TXT};

    #[test]
    fn a_published_value_is_recognized_from_the_record_itself() {
        let record = RData::TXT(TXT::new(vec!["p9JG8PAA".to_owned(), "4Wf7".to_owned()]));
        assert!(published(&record, "p9JG8PAA4Wf7"));
        assert!(!published(&record, "p9JG8PAA"));
        assert!(!published(
            &RData::TXT(TXT::new(vec!["other".to_owned()])),
            "p9JG8PAA4Wf7"
        ));
    }
    #[tokio::test]
    async fn propagation_requeries_long_ttl_answers_and_requires_both_resolvers() {
        use hickory_resolver::proto::serialize::binary::{BinDecodable, BinEncodable};
        use hickory_resolver::{
            config::{NameServerConfig, ResolverConfig},
            proto::{
                op::{Message, ResponseCode},
                rr::{Name, Record, rdata::SOA},
            },
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::net::UdpSocket;

        let mut fixtures = Vec::new();
        for _ in 0..2 {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let address = socket.local_addr().unwrap();
            let mode = Arc::new(AtomicUsize::new(0));
            let queries = Arc::new(AtomicUsize::new(0));
            let response_mode = mode.clone();
            let query_count = queries.clone();
            let task = tokio::spawn(async move {
                let mut buffer = [0; 4096];
                loop {
                    let (size, peer) = socket.recv_from(&mut buffer).await.unwrap();
                    let request = Message::from_bytes(&buffer[..size]).unwrap();
                    query_count.fetch_add(1, Ordering::SeqCst);
                    let mut response =
                        Message::response(request.metadata.id, request.metadata.op_code);
                    response.add_queries(request.queries.clone());
                    response.metadata.authoritative = true;
                    match response_mode.load(Ordering::SeqCst) {
                        0 => {
                            response.metadata.response_code = ResponseCode::NXDomain;
                            let zone = Name::from_ascii("example.test.").unwrap();
                            response.add_authority(Record::from_rdata(
                                zone.clone(),
                                3600,
                                RData::SOA(SOA::new(
                                    zone.clone(),
                                    zone,
                                    1,
                                    3600,
                                    600,
                                    86_400,
                                    3600,
                                )),
                            ));
                        }
                        mode => {
                            let value = if mode == 1 { "wrong" } else { "expected" };
                            response.add_answer(Record::from_rdata(
                                request.queries[0].name().clone(),
                                3600,
                                RData::TXT(TXT::new(vec![value.into()])),
                            ));
                        }
                    }
                    socket
                        .send_to(&response.to_bytes().unwrap(), peer)
                        .await
                        .unwrap();
                }
            });
            let mut server = NameServerConfig::udp(address.ip());
            server.connections[0].port = address.port();
            let resolver =
                super::propagation_resolver(ResolverConfig::from_name_servers(vec![server]))
                    .unwrap();
            fixtures.push((resolver, mode, queries, task));
        }
        let name = "_acme-challenge.example.test.";
        for (resolver, _, _, _) in &fixtures {
            assert!(!super::propagation_visible(resolver, name, "expected").await);
        }
        fixtures[0].1.store(1, Ordering::SeqCst);
        fixtures[1].1.store(2, Ordering::SeqCst);
        let left = super::propagation_visible(&fixtures[0].0, name, "expected").await;
        let right = super::propagation_visible(&fixtures[1].0, name, "expected").await;
        assert!(!left && right);
        fixtures[0].1.store(2, Ordering::SeqCst);
        for (resolver, _, queries, _) in &fixtures {
            assert!(super::propagation_visible(resolver, name, "expected").await);
            assert!(queries.load(Ordering::SeqCst) >= 3);
        }
        for (_, _, _, task) in fixtures {
            task.abort();
        }
    }
}
