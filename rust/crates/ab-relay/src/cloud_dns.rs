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
                let resolver = hickory_resolver::TokioResolver::builder_with_config(
                    config,
                    hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
                )
                .build()
                .map_err(|_| Error::Configuration)?;
                loop {
                    if resolver.txt_lookup(name).await.is_ok_and(|answer| {
                        answer
                            .answers()
                            .iter()
                            .any(|txt| published(&txt.data, value))
                    }) {
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
}
