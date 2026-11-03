use crate::phone::{Gateway, random};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_world::{Error, Result};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub const COOKIE: &str = "ab_session_local";
pub const USER_HANDLE: &[u8] = b"apple-bridge-owner";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Clone)]
pub enum Cbor {
    Int(i64),
    Bytes(Vec<u8>),
    Text(String),
    Map(Vec<(Cbor, Cbor)>),
}

fn head(major: u8, size: usize, out: &mut Vec<u8>) {
    let major = major << 5;
    match u8::try_from(size) {
        Ok(small) if small < 24 => out.push(major | small),
        Ok(small) => out.extend_from_slice(&[major | 0x18, small]),
        Err(_) => {
            let wide = u16::try_from(size).unwrap_or(u16::MAX);
            out.push(major | 0x19);
            out.extend_from_slice(&wide.to_be_bytes());
        }
    }
}

fn sort_key(value: &Cbor) -> (i64, String) {
    match value {
        Cbor::Int(n) => (*n, String::new()),
        Cbor::Text(s) => (0, s.clone()),
        _ => (0, String::new()),
    }
}

impl Cbor {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Cbor::Int(n) if *n >= 0 => head(0, usize::try_from(*n).unwrap_or(0), out),
            Cbor::Int(n) => head(1, usize::try_from(-1 - *n).unwrap_or(0), out),
            Cbor::Bytes(bytes) => {
                head(2, bytes.len(), out);
                out.extend_from_slice(bytes);
            }
            Cbor::Text(text) => {
                head(3, text.len(), out);
                out.extend_from_slice(text.as_bytes());
            }
            Cbor::Map(pairs) => {
                head(5, pairs.len(), out);
                let mut sorted: Vec<_> = pairs.iter().collect();
                sorted.sort_by_key(|(key, _)| sort_key(key));
                for (key, value) in sorted {
                    key.write(out);
                    value.write(out);
                }
            }
        }
    }
}

fn text(value: &str) -> Cbor {
    Cbor::Text(value.to_owned())
}

#[derive(Debug)]
pub struct Authenticator {
    pub user_verified: bool,
    pub counter: u32,
    pub id: Vec<u8>,
    key: SigningKey,
}

impl Authenticator {
    pub fn new() -> Result<Self> {
        let key = SigningKey::from_slice(&random::<32>()?).map_err(|e| invalid(e.to_string()))?;
        Ok(Self {
            user_verified: true,
            counter: 0,
            id: random::<32>()?.to_vec(),
            key,
        })
    }

    fn cose_key(&self) -> Result<Cbor> {
        let point = self.key.verifying_key().to_sec1_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            return Err(invalid("public key coordinates"));
        };
        Ok(Cbor::Map(vec![
            (Cbor::Int(1), Cbor::Int(2)),
            (Cbor::Int(3), Cbor::Int(-7)),
            (Cbor::Int(-1), Cbor::Int(1)),
            (Cbor::Int(-2), Cbor::Bytes(x.to_vec())),
            (Cbor::Int(-3), Cbor::Bytes(y.to_vec())),
        ]))
    }

    pub fn credential(
        &mut self,
        create: bool,
        challenge: &str,
        origin: &str,
        rp: &str,
    ) -> Result<Value> {
        let kind = if create {
            "webauthn.create"
        } else {
            "webauthn.get"
        };
        let client = serde_json::to_vec(&json!({
            "type": kind,
            "challenge": challenge,
            "origin": origin,
            "crossOrigin": false,
        }))?;
        let mut auth = Sha256::digest(rp.as_bytes()).to_vec();
        let mut flags: u8 = if self.user_verified { 5 } else { 1 };
        if create {
            flags |= 64;
        }
        self.counter += 1;
        auth.push(flags);
        auth.extend_from_slice(&self.counter.to_be_bytes());
        let mut response = serde_json::Map::new();
        response.insert("clientDataJSON".into(), json!(encode(&client)));
        if create {
            auth.extend_from_slice(&[0u8; 16]);
            let length = u16::try_from(self.id.len()).map_err(|_| invalid("credential id"))?;
            auth.extend_from_slice(&length.to_be_bytes());
            auth.extend_from_slice(&self.id);
            auth.extend_from_slice(&self.cose_key()?.encode());
            let attestation = Cbor::Map(vec![
                (text("fmt"), text("none")),
                (text("attStmt"), Cbor::Map(vec![])),
                (text("authData"), Cbor::Bytes(auth)),
            ]);
            response.insert(
                "attestationObject".into(),
                json!(encode(&attestation.encode())),
            );
            response.insert("transports".into(), json!(["internal"]));
        } else {
            let mut message = auth.clone();
            message.extend_from_slice(&Sha256::digest(&client));
            let signature: Signature = self.key.sign(&message);
            response.insert("authenticatorData".into(), json!(encode(&auth)));
            response.insert(
                "signature".into(),
                json!(encode(signature.to_der().as_bytes())),
            );
            response.insert("userHandle".into(), json!(encode(USER_HANDLE)));
        }
        Ok(json!({
            "id": encode(&self.id),
            "rawId": encode(&self.id),
            "type": "public-key",
            "response": Value::Object(response),
            "clientExtensionResults": {},
        }))
    }
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    #[must_use]
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
            .find_map(|(_, value)| {
                value
                    .split(';')
                    .next()
                    .and_then(|pair| pair.strip_prefix(&format!("{name}=")))
                    .map(str::to_owned)
            })
    }
}

fn parse(raw: &[u8]) -> Result<Reply> {
    let mut rest = raw;
    loop {
        let split = rest
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| invalid("response head"))?;
        let head = String::from_utf8_lossy(&rest[..split]).into_owned();
        let body = &rest[split + 4..];
        let mut lines = head.split("\r\n");
        let status: u16 = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| invalid("response status"))?;
        if status == 100 {
            rest = body;
            continue;
        }
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
            .collect();
        return Ok(Reply {
            status,
            headers,
            body: body.to_vec(),
        });
    }
}

#[derive(Debug)]
pub struct Owner {
    pub authenticator: Authenticator,
    pub origin: String,
    pub cookie: Option<String>,
}

impl Owner {
    pub fn new(gateway: &Gateway) -> Result<Self> {
        let port = gateway
            .api_address
            .rsplit(':')
            .next()
            .ok_or_else(|| invalid("API port"))?;
        Ok(Self {
            authenticator: Authenticator::new()?,
            origin: format!("http://localhost:{port}"),
            cookie: None,
        })
    }

    pub fn enroll(gateway: &Gateway) -> Result<Self> {
        let mut owner = Self::new(gateway)?;
        let output = gateway.command(&["web", "enroll", "--host", &owner.origin])?;
        let link = output.split_whitespace().next().unwrap_or_default();
        let token = link
            .split_once('#')
            .map(|(_, fragment)| fragment.to_owned())
            .ok_or_else(|| invalid("enrollment link"))?;
        let reply = owner.sign_in("enroll", &token, "trusted")?;
        if reply.status != 200 || owner.cookie.is_none() {
            return Err(invalid(format!("owner enrollment HTTP {}", reply.status)));
        }
        Ok(owner)
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<Reply> {
        let mut command = Command::new("curl");
        command
            .args(["--silent", "--show-error", "--max-time", "15", "-i"])
            .args(["-X", method])
            .args(["-H", &format!("Origin: {}", self.origin)])
            .args(["-H", &format!("Content-Type: {content_type}")]);
        if let Some(cookie) = &self.cookie {
            command.args(["-H", &format!("Cookie: {COOKIE}={cookie}")]);
        }
        command
            .args(["--data-binary", "@-"])
            .arg(format!("{}{path}", self.origin))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(invalid(format!("request exited {}", output.status)));
        }
        parse(&output.stdout)
    }

    pub fn json(&self, method: &str, path: &str, input: &Value) -> Result<Reply> {
        self.request(
            method,
            &format!("/auth/{path}"),
            "application/json",
            &serde_json::to_vec(input)?,
        )
    }

    pub fn sign_in(&mut self, flow: &str, token: &str, kind: &str) -> Result<Reply> {
        let options = self.json(
            "POST",
            &format!("{flow}/options"),
            &json!({"token": token, "kind": kind}),
        )?;
        if options.status != 200 {
            return Ok(options);
        }
        let options = options.json();
        let challenge = options["publicKey"]["challenge"]
            .as_str()
            .ok_or_else(|| invalid("challenge"))?;
        let rp = self
            .origin
            .split("://")
            .nth(1)
            .and_then(|host| host.split(':').next())
            .ok_or_else(|| invalid("relying party"))?
            .to_owned();
        let credential =
            self.authenticator
                .credential(flow != "login", challenge, &self.origin, &rp)?;
        let reply = self.json(
            "POST",
            &format!("{flow}/finish"),
            &json!({"ceremony_id": options["ceremony_id"], "credential": credential}),
        )?;
        if let Some(cookie) = reply.cookie(COOKIE) {
            self.cookie = Some(cookie);
        }
        Ok(reply)
    }

    pub fn login(&mut self) -> Result<Reply> {
        self.cookie = None;
        self.sign_in("login", "", "trusted")
    }

    pub fn decide(&self, id: &str, decision: &str) -> Result<(u16, Option<Value>)> {
        let reply = self.request(
            "POST",
            &format!("/approvals/{id}"),
            "application/x-www-form-urlencoded",
            format!("decision={decision}").as_bytes(),
        )?;
        let page = String::from_utf8_lossy(&reply.body).into_owned();
        let record = page
            .split_once("<pre>")
            .and_then(|(_, rest)| rest.split_once("</pre>"))
            .and_then(|(body, _)| {
                serde_json::from_str(
                    &body
                        .replace("&quot;", "\"")
                        .replace("&#34;", "\"")
                        .replace("&#39;", "'")
                        .replace("&lt;", "<")
                        .replace("&gt;", ">")
                        .replace("&amp;", "&"),
                )
                .ok()
            });
        Ok((reply.status, record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbor_matches_go_encoding() {
        let map = Cbor::Map(vec![
            (Cbor::Int(3), Cbor::Int(-7)),
            (Cbor::Int(1), Cbor::Int(2)),
            (Cbor::Int(-1), Cbor::Bytes(vec![9; 2])),
        ]);
        assert_eq!(
            map.encode(),
            [0xa3, 0x20, 0x42, 9, 9, 0x01, 0x02, 0x03, 0x26]
        );
        assert_eq!(text("fmt").encode(), [0x63, b'f', b'm', b't']);
        assert_eq!(Cbor::Bytes(vec![0; 40]).encode()[..2], [0x58, 40]);
    }

    #[test]
    fn registration_layout() {
        let Ok(mut key) = Authenticator::new() else {
            panic!("authenticator")
        };
        let Ok(value) = key.credential(true, "AAAA", "http://localhost:1", "localhost") else {
            panic!("credential")
        };
        assert_eq!(value["id"], value["rawId"]);
        assert_eq!(value["response"]["transports"], json!(["internal"]));
        let attestation = URL_SAFE_NO_PAD
            .decode(
                value["response"]["attestationObject"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .unwrap_or_default();
        assert_eq!(attestation[0], 0xa3);
        assert_eq!(key.counter, 1);
    }

    #[test]
    fn assertion_signature_verifies() {
        use p256::ecdsa::signature::Verifier;
        let Ok(mut key) = Authenticator::new() else {
            panic!("authenticator")
        };
        let Ok(value) = key.credential(false, "AAAA", "http://localhost:1", "localhost") else {
            panic!("credential")
        };
        let decode = |name: &str| {
            URL_SAFE_NO_PAD
                .decode(value["response"][name].as_str().unwrap_or_default())
                .unwrap_or_default()
        };
        let mut message = decode("authenticatorData");
        message.extend_from_slice(&Sha256::digest(decode("clientDataJSON")));
        let Ok(signature) = Signature::from_der(&decode("signature")) else {
            panic!("signature")
        };
        assert!(key.key.verifying_key().verify(&message, &signature).is_ok());
        assert_eq!(message[32], 5);
    }

    #[test]
    fn reply_parses_cookies_and_skips_continue() {
        let raw = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nSet-Cookie: ab_session_local=abc; Path=/\r\nContent-Length: 2\r\n\r\n{}";
        let Ok(reply) = parse(raw) else {
            panic!("parse")
        };
        assert_eq!(reply.status, 200);
        assert_eq!(reply.cookie(COOKIE).as_deref(), Some("abc"));
        assert_eq!(reply.body, b"{}");
    }
}
