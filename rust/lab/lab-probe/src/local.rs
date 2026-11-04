use crate::{Error, Reply, Result, failed, http};
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub type Handler = Box<dyn FnMut(i128, &Value) + Send>;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const BODY_LIMIT: u64 = 8 << 20;
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Common {
    #[serde(rename = "URL")]
    pub url: String,
    pub host: String,
    pub token: String,
    pub home: String,
    pub collection: String,
    pub proxy_ready: String,
}

#[must_use]
pub fn new_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let millis = u64::try_from(jiff::Timestamp::now().as_millisecond()).unwrap_or_default();
    for index in 0..6 {
        bytes[5 - index] = u8::try_from((millis >> (8 * index)) & 0xff).unwrap_or_default();
    }
    let mut number = u128::from_be_bytes(bytes);
    let mut out = [b'0'; 26];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[usize::try_from(number % 32).unwrap_or_default()];
        number /= 32;
    }
    let text = String::from_utf8_lossy(&out).into_owned();
    if prefix.is_empty() {
        text
    } else {
        format!("{prefix}_{text}")
    }
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[must_use]
pub fn records_digest(records: &[Value]) -> String {
    let mut text = String::new();
    canonical(&Value::Array(records.to_vec()), &mut text);
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[must_use]
pub fn event_id(cursor: &Value) -> String {
    let epoch = cursor["epoch"].as_str().unwrap_or_default();
    let text = format!(
        "{{\"epoch\":{},\"subscription_revision\":{},\"sequence\":{}}}",
        Value::String(epoch.to_owned()),
        cursor["subscription_revision"].as_u64().unwrap_or_default(),
        cursor["sequence"].as_u64().unwrap_or_default()
    );
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text)
}

#[derive(Debug, Clone)]
pub struct Local {
    address: String,
    host: String,
    token: String,
}
impl Local {
    pub fn new(common: &Common) -> Result<Self> {
        let address = common
            .url
            .strip_prefix("http://")
            .map(|rest| rest.trim_end_matches('/').to_owned())
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .ok_or(Error::Invalid("local url"))?;
        Ok(Self {
            address,
            host: common.host.clone(),
            token: common.token.clone(),
        })
    }
    #[must_use]
    pub fn with_token(&self, token: &str) -> Self {
        Self {
            token: token.to_owned(),
            ..self.clone()
        }
    }
    fn open(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        timeout: Duration,
        extra: &[(&str, &str)],
    ) -> Result<TcpStream> {
        let mut tcp = TcpStream::connect(&self.address)?;
        tcp.set_read_timeout(Some(timeout))?;
        tcp.set_write_timeout(Some(timeout))?;
        let content_type = if body.is_empty() {
            String::new()
        } else {
            "Content-Type: application/json\r\n".to_owned()
        };
        let mut lines = String::new();
        for (key, value) in extra {
            lines.push_str(key);
            lines.push_str(": ");
            lines.push_str(value);
            lines.push_str("\r\n");
        }
        let extra = lines;
        let head = format!(
            "{method} {path} HTTP/1.0\r\nHost: {}\r\nAuthorization: Bearer {}\r\n{content_type}{extra}Content-Length: {}\r\n\r\n",
            self.host,
            self.token,
            body.len()
        );
        tcp.write_all(head.as_bytes())?;
        tcp.write_all(body)?;
        tcp.flush()?;
        Ok(tcp)
    }
    pub fn fetch(&self, method: &str, path: &str, body: &[u8], limit: u64) -> Result<Reply> {
        self.fetch_with(method, path, body, &[], limit)
    }
    pub fn fetch_with(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        headers: &[(&str, &str)],
        limit: u64,
    ) -> Result<Reply> {
        let tcp = self.open(method, path, body, TIMEOUT, headers)?;
        let mut raw = Vec::new();
        match tcp.take(limit).read_to_end(&mut raw) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
            Err(error) => return Err(error.into()),
        }
        http::parse(&raw)
    }
    pub fn post(&self, name: &str, value: &Value) -> Result<(u16, Value)> {
        let reply = self.fetch(
            "POST",
            &format!("/v1/op/{name}"),
            &serde_json::to_vec(value)?,
            BODY_LIMIT,
        )?;
        let body = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        Ok((reply.status, body))
    }
    pub fn op(&self, name: &str, value: &Value) -> Result<Value> {
        let (status, body) = self.post(name, value)?;
        if status == 200 {
            return Ok(body);
        }
        Err(Error::Api {
            operation: name.to_owned(),
            status,
            code: body["error"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
        })
    }
    pub fn named(&self, collection: &str, title: &str) -> Result<Vec<Value>> {
        let mut found = Vec::new();
        let mut after = String::new();
        loop {
            let mut query = json!({"collection": collection, "text": title, "limit": 200});
            if !after.is_empty() {
                query["after"] = Value::String(after.clone());
            }
            let page = self.op("records.query", &query)?;
            for record in page["records"].as_array().into_iter().flatten() {
                if record["data"]["title"].as_str() == Some(title) {
                    found.push(record.clone());
                }
            }
            match page["next"].as_str() {
                Some(next) if !next.is_empty() => next.clone_into(&mut after),
                _ => return Ok(found),
            }
        }
    }
    pub fn watch<F>(&self, after: &Value, handler: F) -> Result<Watch>
    where
        F: FnMut(i128, &Value) + Send + 'static,
    {
        let path = format!("/v1/changes?after={}", event_id(after));
        let tcp = self.open("GET", &path, &[], Duration::from_secs(3600), &[])?;
        let closer = tcp.try_clone()?;
        let mut reader = BufReader::new(tcp);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let status: u16 = line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_default();
        if status != 200 {
            return Err(Error::Failed(format!("SSE status {status}")));
        }
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return failed("SSE ended before the body");
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let mut handler = handler;
        let handle = std::thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let at = jiff::Timestamp::now().as_nanosecond();
                let Some(data) = line.trim_end().strip_prefix("data: ") else {
                    continue;
                };
                let Ok(batch) = serde_json::from_str::<Value>(data) else {
                    break;
                };
                handler(at, &batch);
            }
            finished.store(true, Ordering::SeqCst);
        });
        Ok(Watch {
            closer,
            done,
            handle: Some(handle),
        })
    }
}

#[derive(Debug)]
pub struct Watch {
    closer: TcpStream,
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Watch {
    #[must_use]
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
    pub fn close(&mut self) {
        let _ = self.closer.shutdown(Shutdown::Both);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    head.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(format!("{head}{}", String::from_utf8_lossy(&body)));
                socket.write_all(response.as_bytes()).unwrap();
            }
            seen
        });
        (address, handle)
    }
    fn http(status: u16, body: &str) -> String {
        format!(
            "HTTP/1.0 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }
    fn common(address: &str) -> Common {
        Common {
            url: format!("http://{address}"),
            host: address.into(),
            token: "token-1".into(),
            ..Common::default()
        }
    }

    #[test]
    fn ids_are_prefixed_crockford_and_time_ordered() {
        let first = new_id("cmd");
        let second = new_id("");
        assert!(first.starts_with("cmd_"));
        let body = &first[4..];
        assert_eq!(body.len(), 26);
        assert!(body.bytes().all(|c| ALPHABET.contains(&c)));
        assert_eq!(second.len(), 26);
        assert!(body.as_bytes()[0] <= b'7');
    }

    #[test]
    fn digest_ignores_key_order_and_detects_changes() {
        let a = json!([{"id": "a", "data": {"x": 1, "y": [true, null]}}]);
        let b: Value =
            serde_json::from_str(r#"[{"data":{"y":[true,null],"x":1},"id":"a"}]"#).unwrap();
        let c = json!([{"id": "a", "data": {"x": 2, "y": [true, null]}}]);
        let digest = |value: &Value| records_digest(value.as_array().unwrap());
        assert_eq!(digest(&a), digest(&b));
        assert_ne!(digest(&a), digest(&c));
        assert_eq!(digest(&a).len(), 64);
    }

    #[test]
    fn event_id_is_url_safe_base64_of_the_cursor() {
        let id = event_id(&json!({"epoch": "e1", "subscription_revision": 3, "sequence": 9}));
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(id)
            .unwrap();
        assert_eq!(
            String::from_utf8(raw).unwrap(),
            r#"{"epoch":"e1","subscription_revision":3,"sequence":9}"#
        );
    }

    #[test]
    fn op_sends_the_bearer_and_host_and_reports_error_codes() {
        let (address, server) = serve(vec![
            http(200, r#"{"ok":true}"#),
            http(401, r#"{"error":{"code":"unauthorized"}}"#),
        ]);
        let local = Local::new(&common(&address)).unwrap();
        let reply = local.op("records.query", &json!({"limit": 1})).unwrap();
        assert_eq!(reply["ok"], true);
        let error = local.op("reminders.create", &json!({})).unwrap_err();
        assert_eq!(
            error.to_string(),
            "local API reminders.create status 401 code unauthorized"
        );
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("POST /v1/op/records.query HTTP/1.0"));
        assert!(seen[0].contains("Authorization: Bearer token-1"));
        assert!(seen[0].contains(&format!("Host: {address}")));
        assert!(seen[0].ends_with(r#"{"limit":1}"#));
    }

    #[test]
    fn named_pages_and_filters_by_exact_title() {
        let first = json!({"records": [
            {"id": "1", "version": 1, "data": {"title": "Bridge Test"}},
            {"id": "2", "version": 1, "data": {"title": "Bridge Test 2"}}],
            "next": "page-2"});
        let second =
            json!({"records": [{"id": "3", "version": 4, "data": {"title": "Bridge Test"}}]});
        let (address, server) = serve(vec![
            http(200, &first.to_string()),
            http(200, &second.to_string()),
        ]);
        let local = Local::new(&common(&address)).unwrap();
        let found = local.named("reminders.list", "Bridge Test").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1]["id"], "3");
        let seen = server.join().unwrap();
        assert!(seen[0].contains(r#""text":"Bridge Test""#));
        assert!(seen[1].contains(r#""after":"page-2""#));
    }

    #[test]
    fn watch_timestamps_each_data_line_and_stops_on_close() {
        let batch = r#"{"epoch":"e","changes":[{"record":{"data":{"title":"x"}}}]}"#;
        let body = format!(": keepalive\n\ndata: {batch}\n\ndata: not-json\n\n");
        let response = format!("HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\n{body}");
        let (address, server) = serve(vec![response]);
        let local = Local::new(&common(&address)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watch = local
            .watch(
                &json!({"epoch": "e", "subscription_revision": 1, "sequence": 0}),
                move |at, value| {
                    sender.send((at, value.clone())).unwrap();
                },
            )
            .unwrap();
        let (at, value) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(at > 0);
        assert_eq!(value["changes"][0]["record"]["data"]["title"], "x");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !watch.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(watch.finished());
        watch.close();
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("GET /v1/changes?after="));
    }

    #[test]
    fn watch_refuses_a_non_200_stream() {
        let (address, server) = serve(vec![http(401, "{}")]);
        let local = Local::new(&common(&address)).unwrap();
        let error = local.watch(&Value::Null, |_, _| {}).unwrap_err();
        assert_eq!(error.to_string(), "SSE status 401");
        server.join().unwrap();
    }

    #[test]
    fn rejects_non_http_urls() {
        let mut bad = common("127.0.0.1:1");
        bad.url = "https://127.0.0.1:1".into();
        assert!(Local::new(&bad).is_err());
    }
}
