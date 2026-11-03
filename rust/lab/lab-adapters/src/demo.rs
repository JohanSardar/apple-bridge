use lab_demo::Dataset;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub fn load(world: &World, dataset: &Dataset, seconds: u64) -> Result<()> {
    send(world, "/dataset", &dataset.control_json(), seconds)
}

pub fn send(world: &World, route: &str, input: &Value, seconds: u64) -> Result<()> {
    let body = serde_json::to_vec(input)?;
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--max-time"])
        .arg(seconds.to_string())
        .arg("--unix-socket")
        .arg(world.directory.join("sim.sock"))
        .args([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            "-w",
            "\n%{http_code}",
        ])
        .arg(format!("http://sim{route}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Invalid("control stdin".into()))?;
    let writer = std::thread::spawn(move || stdin.write_all(&body));
    let output = child.wait_with_output()?;
    let _ = writer.join();
    let text = String::from_utf8_lossy(&output.stdout);
    let (reply, status) = text.rsplit_once('\n').unwrap_or((&text, ""));
    if output.status.success() && status.trim() == "200" {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "control {route}: {status} {} {}",
            &reply[..reply.len().min(300)],
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}
