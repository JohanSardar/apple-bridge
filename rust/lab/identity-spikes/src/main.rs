#![forbid(unsafe_code)]

use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

fn database(path: &Path, table: &str) -> Value {
    let result =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|db| {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
        });
    match result {
        Ok(count) => json!({"pass": true, "count": count}),
        Err(error) => {
            json!({"pass": false, "sqlite_code": error.sqlite_error().map(|e| e.extended_code)})
        }
    }
}

fn readable(path: &Path) -> Value {
    let result = File::open(path).and_then(|mut f| f.read(&mut [0u8; 1]));
    match result {
        Ok(_) => json!({"pass": true, "exists": true}),
        Err(error) => json!({"pass": false, "errno": error.raw_os_error()}),
    }
}

fn tcc() -> Result<Value, Box<dyn std::error::Error>> {
    let home = PathBuf::from(std::env::var("HOME")?);
    let chat = home.join("Library/Messages/chat.db");
    let attachment = Connection::open_with_flags(&chat, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|db| db.query_row("SELECT filename FROM attachment WHERE filename IS NOT NULL AND filename != '' LIMIT 1", [], |row| row.get::<_, String>(0)));
    let attachment = match attachment {
        Ok(path) => {
            let path = PathBuf::from(path.replacen('~', &home.to_string_lossy(), 1));
            let root = home.join("Library/Messages/Attachments");
            if path.starts_with(&root)
                && !path
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                match path.canonicalize() {
                    Ok(path) if path.starts_with(&root) => readable(&path),
                    _ => json!({"pass": false, "skipped": true}),
                }
            } else {
                json!({"pass": false, "skipped": true})
            }
        }
        Err(error) => {
            json!({"pass": false, "sqlite_code": error.sqlite_error().map(|e| e.extended_code)})
        }
    };
    let biome =
        match std::fs::read_dir(home.join("Library/Biome/streams/restricted/App.InFocus/local")) {
            Ok(entries) => match entries.collect::<Result<Vec<_>, _>>() {
                Ok(entries) => json!({"pass": true, "count": entries.len()}),
                Err(error) => json!({"pass": false, "errno": error.raw_os_error()}),
            },
            Err(error) => json!({"pass": false, "errno": error.raw_os_error()}),
        };
    Ok(json!({
        "pid": std::process::id(), "ppid": ab_sys::parent_pid(), "responsible_pid": ab_sys::responsible_pid(),
        "chat": database(&chat, "message"),
        "wal": readable(&home.join("Library/Messages/chat.db-wal")),
        "attachment": attachment,
        "notes": database(&home.join("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite"), "ZICCLOUDSYNCINGOBJECT"),
        "biome": biome,
        "safari": database(&home.join("Library/Safari/History.db"), "history_items")
    }))
}

fn keychain(path: &Path) -> Value {
    use security_framework::os::macos::{keychain::SecKeychain, passwords::find_generic_password};
    let result = (|| {
        let _guard = SecKeychain::disable_user_interaction()?;
        let keychain = SecKeychain::open(path)?;
        let (password, _) =
            find_generic_password(Some(&[keychain]), "apple-bridge-spike", "spike")?;
        Ok::<_, security_framework::base::Error>(
            password.as_ref() == b"00112233445566778899aabbccddeeff",
        )
    })();
    match result {
        Ok(pass) => json!({"pass": pass, "status": 0}),
        Err(error) => json!({"pass": false, "status": error.code()}),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let report = if args.get(1).map(String::as_str) == Some("keychain") {
        keychain(Path::new(args.get(2).ok_or("missing test keychain")?))
    } else {
        tcc()?
    };
    let mut bytes = serde_json::to_vec(&report)?;
    bytes.push(b'\n');
    if let Some(fd) = args.get(1).and_then(|s| s.parse().ok()) {
        ab_sys::write_report(fd, &bytes)?;
    } else {
        use std::io::Write;
        std::io::stdout().write_all(&bytes)?;
    }
    Ok(())
}
