use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    process::{Command, Stdio},
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
            .args(["--silent", "--show-error", "--max-time", &seconds.to_string()])
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
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(body) = body {
                stdin.write_all(body)?;
            }
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
        let body = serde_json::to_vec(input)?;
        let reply = self.request(
            "POST",
            &format!("/v1/op/{operation}"),
            &[("Content-Type", "application/json")],
            Some(&body),
            seconds,
        )?;
        accepted(&reply, serde_json::from_slice(&reply.body)?)
    }
    pub fn call_mcp(&self, operation: &str, input: &Value, seconds: u64) -> Result<Option<Value>> {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": operation.replace('.', "_"), "arguments": input}
        }))?;
        let reply = self.request(
            "POST",
            "/mcp",
            &[
                ("Content-Type", "application/json"),
                ("Accept", "application/json, text/event-stream"),
                ("MCP-Protocol-Version", "2025-06-18"),
            ],
            Some(&body),
            seconds,
        )?;
        let text = String::from_utf8(reply.body.clone())
            .map_err(|_| Error::Invalid("non UTF-8 MCP body".into()))?;
        let envelope = match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => {
                let messages = lab_wire::sse_messages(&text)
                    .map_err(|e| Error::Invalid(format!("MCP stream: {e}")))?;
                let message = messages
                    .iter()
                    .find(|m| m.result.is_some() || m.error.is_some())
                    .ok_or_else(|| Error::Invalid("MCP stream without response".into()))?;
                serde_json::to_value(message)?
            }
        };
        let content = envelope["result"]["structuredContent"].clone();
        if content.is_null() {
            return Ok(None);
        }
        accepted(&reply, content)
    }
}
fn accepted(reply: &Reply, value: Value) -> Result<Option<Value>> {
    if !value["error"].is_null() || reply.status != 200 {
        return Ok(None);
    }
    Ok(Some(value))
}
