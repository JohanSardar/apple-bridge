use crate::{Error, Result, http::DnsProvider};
use hickory_resolver::proto::{
    op::{Message, MessageType, OpCode, ResponseCode},
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, NS, SOA, TXT},
    },
    serialize::binary::{BinDecodable, BinEncodable},
};
use std::{
    collections::HashMap,
    future::Future,
    net::Ipv4Addr,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};

const TTL: u32 = 60;
const CHALLENGE_LIFETIME: Duration = Duration::from_hours(1);

#[derive(Debug)]
struct Zone {
    name: Name,
    nameserver: Name,
    address: Ipv4Addr,
    challenges: Mutex<HashMap<String, Vec<(String, Instant)>>>,
}
#[derive(Debug)]
pub struct Server(Arc<Zone>);

impl Server {
    pub async fn start(listen: &str, zone: &str, address: Ipv4Addr) -> Result<Arc<Self>> {
        let name = Name::from_ascii(format!("{}.", zone.trim_end_matches('.')))
            .map_err(|_| Error::Configuration)?;
        let nameserver = Name::from_ascii("ns1")
            .and_then(|label| label.append_domain(&name))
            .map_err(|_| Error::Configuration)?;
        let shared = Arc::new(Zone {
            name,
            nameserver,
            address,
            challenges: Mutex::default(),
        });
        let udp = UdpSocket::bind(listen)
            .await
            .map_err(|_| Error::Configuration)?;
        let tcp = TcpListener::bind(listen)
            .await
            .map_err(|_| Error::Configuration)?;
        tokio::spawn(serve_udp(udp, shared.clone()));
        tokio::spawn(serve_tcp(tcp, shared.clone()));
        Ok(Arc::new(Self(shared)))
    }
    pub fn bound(zone: &str, address: Ipv4Addr) -> Result<Self> {
        let name = Name::from_ascii(format!("{}.", zone.trim_end_matches('.')))
            .map_err(|_| Error::Configuration)?;
        let nameserver = Name::from_ascii("ns1")
            .and_then(|label| label.append_domain(&name))
            .map_err(|_| Error::Configuration)?;
        Ok(Self(Arc::new(Zone {
            name,
            nameserver,
            address,
            challenges: Mutex::default(),
        })))
    }
    pub fn answer(&self, query: &[u8]) -> Option<Vec<u8>> {
        self.0.answer(query)
    }
}

impl DnsProvider for Server {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let key = format!("{}.", name.trim_end_matches('.')).to_ascii_lowercase();
            let parsed = Name::from_ascii(&key).map_err(|_| Error::Denied)?;
            if !self.0.name.zone_of(&parsed) {
                return Err(Error::Denied);
            }
            let mut challenges = self.0.challenges.lock().map_err(|_| Error::Denied)?;
            if value.is_empty() {
                challenges.remove(&key);
                return Ok(());
            }
            let now = Instant::now();
            challenges.retain(|_, values| {
                values.retain(|(_, until)| *until > now);
                !values.is_empty()
            });
            if challenges.len() >= 4096 {
                return Err(Error::Denied);
            }
            let values = challenges.entry(key).or_default();
            if values.len() >= 4 {
                values.remove(0);
            }
            values.push((value.to_owned(), now + CHALLENGE_LIFETIME));
            Ok(())
        })
    }
}

impl Zone {
    fn soa(&self) -> Record {
        let admin = Name::from_ascii("hostmaster")
            .and_then(|label| label.append_domain(&self.name))
            .unwrap_or_else(|_| self.name.clone());
        Record::from_rdata(
            self.name.clone(),
            TTL,
            RData::SOA(SOA::new(
                self.nameserver.clone(),
                admin,
                1,
                3600,
                600,
                86_400,
                TTL,
            )),
        )
    }
    fn answers(&self, name: &Name, kind: RecordType) -> Vec<Record> {
        match kind {
            RecordType::A => vec![Record::from_rdata(
                name.clone(),
                TTL,
                RData::A(A(self.address)),
            )],
            RecordType::NS if *name == self.name => vec![Record::from_rdata(
                name.clone(),
                TTL,
                RData::NS(NS(self.nameserver.clone())),
            )],
            RecordType::SOA if *name == self.name => vec![self.soa()],
            RecordType::TXT => {
                let key = name.to_ascii().to_ascii_lowercase();
                let now = Instant::now();
                self.challenges
                    .lock()
                    .ok()
                    .and_then(|challenges| {
                        challenges.get(&key).map(|values| {
                            values
                                .iter()
                                .filter(|(_, until)| *until > now)
                                .map(|(value, _)| {
                                    Record::from_rdata(
                                        name.clone(),
                                        TTL,
                                        RData::TXT(TXT::new(vec![value.clone()])),
                                    )
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
    fn answer(&self, query: &[u8]) -> Option<Vec<u8>> {
        let request = Message::from_bytes(query).ok()?;
        if request.metadata.message_type != MessageType::Query {
            return None;
        }
        let mut response = Message::response(request.metadata.id, request.metadata.op_code);
        response.add_queries(request.queries.clone());
        let Some(question) = request.queries.first() else {
            response.metadata.response_code = ResponseCode::FormErr;
            return response.to_bytes().ok();
        };
        if request.metadata.op_code != OpCode::Query {
            response.metadata.response_code = ResponseCode::NotImp;
            return response.to_bytes().ok();
        }
        let name = question.name();
        if !self.name.zone_of(name) {
            response.metadata.response_code = ResponseCode::Refused;
            return response.to_bytes().ok();
        }
        response.metadata.authoritative = true;
        let records = self.answers(name, question.query_type());
        if records.is_empty() {
            response.add_authority(self.soa());
        } else {
            response.add_answers(records);
        }
        response.to_bytes().ok()
    }
}

async fn serve_udp(socket: UdpSocket, zone: Arc<Zone>) {
    let mut buffer = [0u8; 1500];
    loop {
        let Ok((size, peer)) = socket.recv_from(&mut buffer).await else {
            continue;
        };
        if let Some(reply) = zone.answer(&buffer[..size]) {
            let _ = socket.send_to(&reply, peer).await;
        }
    }
}

async fn serve_tcp(listener: TcpListener, zone: Arc<Zone>) {
    let slots = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let zone = zone.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let exchange = async {
                let size = usize::from(stream.read_u16().await?);
                let mut query = vec![0; size];
                stream.read_exact(&mut query).await?;
                if let Some(reply) = zone.answer(&query) {
                    let length = u16::try_from(reply.len()).map_err(std::io::Error::other)?;
                    stream.write_u16(length).await?;
                    stream.write_all(&reply).await?;
                }
                Ok::<_, std::io::Error>(())
            };
            let _ = tokio::time::timeout(Duration::from_secs(5), exchange).await;
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use hickory_resolver::proto::op::Query;

    fn ask(server: &Server, name: &str, kind: RecordType) -> Message {
        let mut message = Message::query();
        message.metadata.id = 7;
        message.add_query(Query::query(Name::from_ascii(name).unwrap(), kind));
        Message::from_bytes(&server.answer(&message.to_bytes().unwrap()).unwrap()).unwrap()
    }
    fn zone() -> Server {
        Server::bound("bridge.example.com", Ipv4Addr::new(203, 0, 113, 9)).unwrap()
    }

    #[test]
    fn every_name_in_the_zone_is_the_relay() {
        let server = zone();
        for name in [
            "bridge.example.com.",
            "ns1.bridge.example.com.",
            "0123456789abcdef.bridge.example.com.",
        ] {
            let reply = ask(&server, name, RecordType::A);
            assert_eq!(reply.metadata.response_code, ResponseCode::NoError);
            assert!(reply.metadata.authoritative);
            assert_eq!(reply.answers.len(), 1, "{name}");
        }
    }

    #[test]
    fn other_zones_are_refused() {
        let reply = ask(&zone(), "lexaai.tech.", RecordType::A);
        assert_eq!(reply.metadata.response_code, ResponseCode::Refused);
        assert_eq!(reply.answers.len(), 0);
    }

    #[tokio::test]
    async fn challenges_are_published_at_once_and_only_inside_the_zone() {
        let server = zone();
        let name = "_acme-challenge.0123456789abcdef.bridge.example.com";
        let empty = ask(&server, &format!("{name}."), RecordType::TXT);
        assert_eq!(empty.answers.len(), 0);
        assert_eq!(
            empty.authorities.len(),
            1,
            "negative answer carries the SOA"
        );
        server.txt(name, "token-value").await.unwrap();
        let reply = ask(&server, &format!("{name}."), RecordType::TXT);
        assert_eq!(reply.answers.len(), 1);
        server.txt(name, "").await.unwrap();
        assert_eq!(
            ask(&server, &format!("{name}."), RecordType::TXT)
                .answers
                .len(),
            0,
            "an empty value clears it"
        );
        assert!(
            server
                .txt("_acme-challenge.other.example.org", "x")
                .await
                .is_err()
        );
    }
}
