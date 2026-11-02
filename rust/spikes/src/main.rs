#![forbid(unsafe_code)]
mod tls;
use ab_wire::gojson::{self, CommandHash, EscapeMode, GoTime};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{self, BufRead, Write};

#[derive(Deserialize)]
struct Request {
    raw: String,
    #[serde(default)]
    raw_bytes: Vec<u8>,
    #[serde(default)]
    command: bool,
    #[serde(default)]
    nohtml: bool,
}
#[derive(Deserialize)]
struct Command {
    op: String,
    args: Box<serde_json::value::RawValue>,
    #[serde(default)]
    expected_version: u64,
    #[serde(default = "zero_time")]
    expires_at: String,
    #[serde(default)]
    admission_era: String,
}
fn zero_time() -> String {
    "0001-01-01T00:00:00Z".into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = std::env::args().nth(1) {
        if path == "tls" {
            return tls::run();
        }
        return sqlite(&path);
    }
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let req: Request = serde_json::from_str(&line?)?;
        let digest = if req.command {
            let c: Command = serde_json::from_str(&req.raw)?;
            CommandHash {
                op: &c.op,
                args: c.args.get().as_bytes(),
                version: c.expected_version,
                expires: GoTime::parse(&c.expires_at)?,
                era: &c.admission_era,
            }
            .digest()?
        } else {
            let mode = if req.nohtml {
                EscapeMode::NoHtml
            } else {
                EscapeMode::Html
            };
            format!(
                "{:x}",
                Sha256::digest(gojson::record_data(
                    if req.raw_bytes.is_empty() {
                        req.raw.as_bytes()
                    } else {
                        &req.raw_bytes
                    },
                    mode
                )?)
            )
        };
        writeln!(out, "{digest}")?;
        out.flush()?;
    }
    Ok(())
}
fn sqlite(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db = rusqlite::Connection::open(path)?;
    let version: String = db.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let opts = db
        .prepare("PRAGMA compile_options")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    writeln!(
        io::stdout(),
        "version={version} fts5={} omit_wal={}",
        opts.iter().any(|s| s == "ENABLE_FTS5"),
        opts.iter().any(|s| s == "OMIT_WAL")
    )?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS spike(n INTEGER); INSERT INTO spike VALUES(2);")?;
    let reader = rusqlite::Connection::open(path)?;
    reader.execute_batch("BEGIN")?;
    let before: i64 = reader.query_row("SELECT count(*) FROM spike", [], |r| r.get(0))?;
    db.execute("INSERT INTO spike VALUES(3)", [])?;
    let during: i64 = reader.query_row("SELECT count(*) FROM spike", [], |r| r.get(0))?;
    reader.execute_batch("COMMIT")?;
    let after: i64 = reader.query_row("SELECT count(*) FROM spike", [], |r| r.get(0))?;
    if before != during || after != before + 1 {
        return Err("snapshot mismatch".into());
    }
    let mut stmt = db.prepare("SELECT rowid FROM corpus WHERE corpus MATCH ? ORDER BY rowid")?;
    for query in [
        "cafe",
        "hello",
        "\"hello world\"",
        "hel*",
        "日本語",
        "resume",
        "alpha OR beta",
        "NOTHING",
    ] {
        let rows = stmt
            .query_map([query], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        writeln!(io::stdout(), "{}", serde_json::to_string(&rows)?)?;
    }
    db.execute_batch(
        "INSERT INTO corpus VALUES('Rust hello café'); PRAGMA wal_checkpoint(TRUNCATE);",
    )?;
    Ok(())
}
