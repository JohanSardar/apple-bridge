use crate::assertions::await_until;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
};

pub(crate) const HOST: &str = "ubuntu";

pub(crate) fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn ssh_command(script: &str) -> Command {
    let mut command = Command::new("ssh");
    command.args(["-o", "BatchMode=yes", HOST, script]);
    command
}
pub(crate) fn remote(script: &str) -> Result<String> {
    let output: Output = ssh_command(script).stdin(Stdio::null()).output()?;
    if !output.status.success() {
        return Err(invalid(format!("ubuntu command exited {}", output.status)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
fn environment(arguments: &[&str]) -> String {
    let mut text = String::from("env APPLE_BRIDGE_SECRET_STORE=file");
    for argument in arguments {
        text.push(' ');
        text.push_str(&quote(argument));
    }
    text
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    pub url: String,
    pub host: String,
    pub token: String,
    pub home: String,
}

#[derive(Debug)]
pub(crate) struct RelayHost {
    pub directory: String,
    pub replica: Option<Endpoint>,
    bridge: String,
    probe: String,
    logs: PathBuf,
    spawned: Vec<(Child, String)>,
}

fn require_file(name: &str) -> Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            invalid(format!(
                "{name} must name a Linux binary built on ssh build"
            ))
        })
}

impl RelayHost {
    pub(crate) fn open(world: &World) -> Result<Self> {
        let stamp = jiff::Timestamp::now().as_millisecond();
        let directory = format!("/tmp/ablab-relay-{}-{stamp}", std::process::id());
        remote(&format!(
            "umask 077; mkdir -p {0}/replica {0}/route",
            quote(&directory)
        ))?;
        Ok(Self {
            bridge: format!("{directory}/bridge-linux-amd64"),
            probe: format!("{directory}/probe"),
            directory,
            replica: None,
            logs: world.directory.clone(),
            spawned: Vec::new(),
        })
    }
    pub(crate) fn upload(&self) -> Result<()> {
        self.copy(&[
            (require_file("AB_LAB_REMOTE_BRIDGE")?, &self.bridge),
            (require_file("AB_LAB_PROBE")?, &self.probe),
        ])
    }
    pub(crate) fn upload_probe(&self) -> Result<()> {
        self.copy(&[(require_file("AB_LAB_PROBE")?, &self.probe)])
    }
    #[allow(clippy::unused_self)]
    fn copy(&self, files: &[(PathBuf, &String)]) -> Result<()> {
        for (local, remote_path) in files {
            let status = Command::new("scp")
                .args(["-o", "BatchMode=yes"])
                .arg(local)
                .arg(format!("{HOST}:{remote_path}"))
                .stdin(Stdio::null())
                .status()?;
            if !status.success() {
                return Err(invalid("copying a binary to ubuntu failed"));
            }
        }
        let names: Vec<String> = files.iter().map(|(_, path)| quote(path)).collect();
        remote(&format!("chmod 700 {}", names.join(" ")))?;
        Ok(())
    }
    #[allow(clippy::unused_self)]
    pub(crate) fn run(&self, arguments: &[&str]) -> Result<String> {
        remote(&environment(arguments))
    }
    #[allow(clippy::unused_self)]
    pub(crate) fn read_json(&self, path: &str) -> Option<Value> {
        let text = remote(&format!("cat {} 2>/dev/null", quote(path))).ok()?;
        serde_json::from_str(&text).ok()
    }
    fn spawn(&mut self, name: &str, home: &str, arguments: &[&str]) -> Result<()> {
        let script = format!(
            "umask 077; echo $$ > {}; exec {}",
            quote(&format!("{home}/pid")),
            environment(arguments)
        );
        let log = fs::File::create(self.logs.join(format!("{name}.log")))?;
        let child = ssh_command(&script)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        self.spawned.push((child, home.to_owned()));
        Ok(())
    }
    pub(crate) fn start_replica(
        &mut self,
        pair_code: &str,
        address: &str,
        public_ip: &str,
        scope: &str,
    ) -> Result<()> {
        let route_home = format!("{}/route", self.directory);
        let home = format!("{}/replica", self.directory);
        let ready = format!("{route_home}/ready.json");
        let probe = self.probe.clone();
        self.spawn(
            "relay-route",
            &route_home,
            &[
                &probe, "proxy", "--host", address, "--ip", public_ip, "--ready", &ready,
            ],
        )?;
        let mut proxy = String::new();
        let routed = await_until(10, || {
            proxy = self
                .read_json(&ready)
                .and_then(|value| value["url"].as_str().map(str::to_owned))
                .unwrap_or_default();
            !proxy.is_empty()
        });
        if !routed {
            return Err(invalid("relay route did not become ready"));
        }
        let bridge = self.bridge.clone();
        let through = |tail: &[&str]| -> Vec<String> {
            let mut all: Vec<String> = ["HTTPS_PROXY", "https_proxy"]
                .iter()
                .map(|name| format!("{name}={proxy}"))
                .collect();
            all.extend([
                "NO_PROXY=".to_owned(),
                "no_proxy=".to_owned(),
                bridge.clone(),
            ]);
            all.extend(tail.iter().map(|part| (*part).to_owned()));
            all
        };
        let join = through(&["join", pair_code, "--home", &home]);
        self.run(&join.iter().map(String::as_str).collect::<Vec<_>>())?;
        let token = self
            .run(&[
                &self.bridge,
                "token",
                "create",
                "--home",
                &home,
                "--scope",
                scope,
            ])?
            .trim()
            .to_owned();
        let start = through(&[
            "start",
            "--role",
            "replica",
            "--home",
            &home,
            "--api-addr",
            "127.0.0.1:0",
            "--blob-cap",
            "0",
        ]);
        let mut arguments: Vec<&str> = vec!["env"];
        arguments.extend(start.iter().map(String::as_str));
        self.spawn("relay-replica", &home, &arguments)?;
        let mut api = String::new();
        let started = await_until(20, || {
            api = self
                .read_json(&format!("{home}/runtime.json"))
                .and_then(|value| value["api_address"].as_str().map(str::to_owned))
                .unwrap_or_default();
            !api.is_empty()
        });
        if !started {
            return Err(invalid("relay replica did not start"));
        }
        self.replica = Some(Endpoint {
            url: format!("http://{api}"),
            host: api,
            token,
            home,
        });
        Ok(())
    }
    pub(crate) fn input(&self) -> Result<Value> {
        let replica = self
            .replica
            .as_ref()
            .ok_or_else(|| invalid("no relay replica"))?;
        Ok(serde_json::json!({
            "URL": replica.url, "Host": replica.host, "Token": replica.token, "Home": replica.home,
            "ProxyReady": format!("{}/route/ready.json", self.directory), "Bridge": self.bridge,
        }))
    }
    pub(crate) fn probe(&self, action: &str, input: &Value) -> Result<Value> {
        let mut child = ssh_command(&environment(&[&self.probe, action]))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.to_string().as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "relay probe {action} exited {}",
                output.status
            )));
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    pub(crate) fn exchange(
        &self,
        action: &str,
        input: &Value,
        next: &mut dyn FnMut() -> Result<Value>,
    ) -> Result<Value> {
        let mut child = ssh_command(&environment(&[&self.probe, action]))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let outcome = (|| {
            let mut stdin = child.stdin.take().ok_or_else(|| invalid("probe stdin"))?;
            let mut stdout =
                BufReader::new(child.stdout.take().ok_or_else(|| invalid("probe stdout"))?);
            let mut read = || -> Result<Value> {
                let mut line = String::new();
                stdout.read_line(&mut line)?;
                let value: Value = serde_json::from_str(&line)?;
                match value["error"].as_str() {
                    Some(message) => Err(invalid(format!("relay probe failed: {message}"))),
                    None => Ok(value),
                }
            };
            writeln!(stdin, "{input}")?;
            if read()?["ready"] != true {
                return Err(invalid("relay probe not ready"));
            }
            let value = next()?;
            writeln!(stdin, "{value}")?;
            stdin.flush()?;
            read()
        })();
        let _ = child.kill();
        let _ = child.wait();
        outcome
    }
    pub(crate) fn stop(&mut self) {
        for (mut child, home) in self.spawned.drain(..).rev() {
            let pid = quote(&format!("{home}/pid"));
            let _ = remote(&format!(
                "if test -f {pid}; then kill -CONT $(cat {pid}); kill -TERM $(cat {pid}); fi"
            ));
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.directory.is_empty() {
            let _ = remote(&format!("rm -rf {}", quote(&self.directory)));
        }
    }
}
impl Drop for RelayHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{environment, quote};

    #[test]
    fn quotes_single_quotes_for_the_remote_shell() {
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn builds_the_file_store_environment_command() {
        assert_eq!(
            environment(&["/tmp/x/probe", "query"]),
            "env APPLE_BRIDGE_SECRET_STORE=file '/tmp/x/probe' 'query'"
        );
    }
}
