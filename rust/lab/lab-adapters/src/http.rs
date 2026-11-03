use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    process::{Child, Command, Stdio},
};

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}
#[derive(Debug)]
pub struct Client {
    address: String,
    token: String,
}
pub fn random_id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 12];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}
impl Client {
    pub fn new(world: &World, scope: &str) -> Result<Self> {
        let address = world.runtime["api_address"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime API address".into()))?
            .to_owned();
        let token = world
            .gateway_cli(&["token", "create", "--scope", scope])?
            .trim()
            .to_owned();
        Ok(Self { address, token })
    }
    pub fn with_token(world: &World, token: &str) -> Result<Self> {
        let address = world.runtime["api_address"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime API address".into()))?
            .to_owned();
        Ok(Self {
            address,
            token: token.to_owned(),
        })
    }
    pub fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        seconds: u64,
    ) -> Result<Reply> {
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--max-time",
                &seconds.to_string(),
            ])
            .args(["-X", method, "-H"])
            .arg(format!("Authorization: Bearer {}", self.token));
        for (name, value) in headers {
            command.args(["-H", &format!("{name}: {value}")]);
        }
        if body.is_some() {
            command.args(["--data-binary", "@-"]);
        }
        command
            .args(["-w", "%{http_code}"])
            .arg(format!("http://{}{path}", self.address))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = command.spawn()?;
        if let (Some(mut stdin), Some(body)) = (child.stdin.take(), body) {
            stdin.write_all(body)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() || output.stdout.len() < 3 {
            return Err(Error::Invalid(format!("request exited {}", output.status)));
        }
        let split = output.stdout.len() - 3;
        let status = std::str::from_utf8(&output.stdout[split..])
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| Error::Invalid("status".into()))?;
        Ok(Reply {
            status,
            body: output.stdout[..split].to_vec(),
        })
    }
    pub fn call(&self, operation: &str, input: &Value, seconds: u64) -> Result<Option<Value>> {
        Ok(self
            .call_raw(operation, input, seconds)?
            .and_then(|(status, value)| accepted(status, value)))
    }
    pub fn call_raw(
        &self,
        operation: &str,
        input: &Value,
        seconds: u64,
    ) -> Result<Option<(u16, Value)>> {
        let body = serde_json::to_vec(input)?;
        let reply = self.request(
            "POST",
            &format!("/v1/op/{operation}"),
            &[("Content-Type", "application/json")],
            Some(&body),
            seconds,
        )?;
        let value = serde_json::from_slice(&reply.body).map_err(|e| {
            Error::Invalid(format!(
                "response {} {e}: {}",
                reply.status,
                String::from_utf8_lossy(&reply.body[..reply.body.len().min(80)])
            ))
        })?;
        Ok(Some((reply.status, value)))
    }
    pub fn spawn_call(&self, operation: &str, input: &Value, mcp: bool) -> Result<Child> {
        let (path, headers, body) = if mcp {
            (
                "/mcp".to_owned(),
                MCP_HEADERS.to_vec(),
                mcp_body(operation, input)?,
            )
        } else {
            (
                format!("/v1/op/{operation}"),
                vec![("Content-Type", "application/json")],
                serde_json::to_vec(input)?,
            )
        };
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
            ])
            .arg(format!("Authorization: Bearer {}", self.token));
        for (name, value) in headers {
            command.args(["-H", &format!("{name}: {value}")]);
        }
        command
            .args(["--data-binary", "@-"])
            .arg(format!("http://{}{path}", self.address))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&body)?;
        }
        Ok(child)
    }
    pub fn call_mcp(&self, operation: &str, input: &Value, seconds: u64) -> Result<Option<Value>> {
        Ok(self
            .call_mcp_raw(operation, input, seconds)?
            .and_then(|(status, value)| accepted(status, value)))
    }
    pub fn call_mcp_raw(
        &self,
        operation: &str,
        input: &Value,
        seconds: u64,
    ) -> Result<Option<(u16, Value)>> {
        let body = mcp_body(operation, input)?;
        let reply = self.request("POST", "/mcp", &MCP_HEADERS, Some(&body), seconds)?;
        let text = String::from_utf8(reply.body.clone())
            .map_err(|_| Error::Invalid("non UTF-8 MCP body".into()))?;
        let envelope = if let Ok(value) = serde_json::from_str::<Value>(&text) {
            value
        } else {
            let messages = lab_wire::sse_messages(&text)
                .map_err(|e| Error::Invalid(format!("MCP stream: {e}")))?;
            let message = messages
                .iter()
                .find(|m| m.result.is_some() || m.error.is_some())
                .ok_or_else(|| Error::Invalid("MCP stream without response".into()))?;
            serde_json::to_value(message)?
        };
        let content = envelope["result"]["structuredContent"].clone();
        if content.is_null() {
            return Ok(None);
        }
        Ok(Some((reply.status, content)))
    }
}
const MCP_HEADERS: [(&str, &str); 3] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("MCP-Protocol-Version", "2025-06-18"),
];
fn mcp_body(operation: &str, input: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": operation.replace('.', "_"), "arguments": input}
    }))?)
}
fn accepted(status: u16, value: Value) -> Option<Value> {
    (value["error"].is_null() && status == 200).then_some(value)
}
