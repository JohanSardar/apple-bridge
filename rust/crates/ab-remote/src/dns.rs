use crate::{Error, Result};
use hickory_resolver::{
    TokioResolver,
    config::{NameServerConfig, ResolverConfig},
    net::runtime::TokioRuntimeProvider,
    proto::rr::{RData, RecordType},
};
use std::{future::Future, net::SocketAddr, pin::Pin};

pub type CnameFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

pub trait Resolver: std::fmt::Debug + Send + Sync {
    fn cname<'a>(&'a self, name: &'a str) -> CnameFuture<'a>;
}

#[derive(Debug)]
pub struct Hickory(TokioResolver);

impl Hickory {
    pub fn system() -> Result<Self> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|_| Error::Contract("DNS resolver unavailable"))?
            .build()
            .map_err(|_| Error::Contract("DNS resolver unavailable"))?;
        Ok(Self(resolver))
    }

    pub fn server(address: SocketAddr) -> Result<Self> {
        let mut server = NameServerConfig::udp(address.ip());
        for connection in &mut server.connections {
            connection.port = address.port();
        }
        let config = ResolverConfig::from_parts(None, vec![], vec![server]);
        let resolver = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
            .build()
            .map_err(|_| Error::Contract("DNS resolver unavailable"))?;
        Ok(Self(resolver))
    }
}

impl Resolver for Hickory {
    fn cname<'a>(&'a self, name: &'a str) -> CnameFuture<'a> {
        Box::pin(async move {
            let answer = self
                .0
                .lookup(name, RecordType::CNAME)
                .await
                .map_err(|_| Error::Contract("DNS lookup failed"))?;
            answer
                .answers()
                .iter()
                .find_map(|record| match &record.data {
                    RData::CNAME(target) => Some(target.to_string()),
                    _ => None,
                })
                .ok_or(Error::Contract("DNS record missing"))
        })
    }
}

pub async fn missing_records(resolver: &dyn Resolver, domain: &str, host: &str) -> Vec<String> {
    let mut missing = Vec::new();
    for (name, target) in [
        (domain.to_owned(), host.to_owned()),
        (
            format!("_acme-challenge.{domain}"),
            format!("_acme-challenge.{host}"),
        ),
    ] {
        let found = resolver
            .cname(&name)
            .await
            .is_ok_and(|got| got.trim_end_matches('.').eq_ignore_ascii_case(&target));
        if !found {
            missing.push(format!("{name} CNAME {target}."));
        }
    }
    missing
}
