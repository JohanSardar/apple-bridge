pub fn now() -> jiff::Timestamp {
    #[cfg(feature = "lab")]
    if let Some(decisions) = decisions() {
        return decisions["now"]
            .as_str()
            .unwrap_or_else(|| panic!("invalid lab replay clock"))
            .parse()
            .unwrap_or_else(|_| panic!("invalid lab replay clock"));
    }
    jiff::Timestamp::now()
}

#[cfg(feature = "lab")]
static COMMAND_CURSOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn command_id() -> Option<String> {
    #[cfg(feature = "lab")]
    if let Some(decisions) = decisions() {
        let index = COMMAND_CURSOR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let id = decisions["command_ids"]
            .get(index)
            .and_then(serde_json::Value::as_str)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c))
            })
            .unwrap_or_else(|| panic!("lab replay command id missing"));
        return Some(id.to_owned());
    }
    None
}

#[cfg(feature = "lab")]
fn decisions() -> Option<serde_json::Value> {
    let path = std::env::var_os("APPLE_BRIDGE_LAB_REPLAY")?;
    let raw = std::fs::read(path).unwrap_or_else(|_| panic!("lab replay decisions unavailable"));
    let value: serde_json::Value =
        serde_json::from_slice(&raw).unwrap_or_else(|_| panic!("invalid lab replay decisions"));
    let _: jiff::Timestamp = value["now"]
        .as_str()
        .unwrap_or_else(|| panic!("invalid lab replay clock"))
        .parse()
        .unwrap_or_else(|_| panic!("invalid lab replay clock"));
    Some(value)
}

pub(crate) fn record_id(
    source: &str,
    generation: &str,
    collection: &str,
    native: &str,
) -> Option<String> {
    #[cfg(feature = "lab")]
    if let Some(decisions) = decisions() {
        let key = format!("{source}\0{generation}\0{collection}\0{native}");
        let id = decisions["aliases"][key]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| panic!("lab replay alias missing"));
        return Some(id.to_owned());
    }
    let _ = (source, generation, collection, native);
    None
}

#[cfg(all(test, feature = "lab"))]
mod tests {
    #[test]
    fn child() {
        if std::env::var_os("REPLAY_DECISION_TEST").is_none() {
            return;
        }
        assert_eq!(super::now().to_string(), "2026-11-03T12:00:00.123456789Z");
        assert_eq!(
            super::record_id("reminders", "generation", "reminders.list", "native").as_deref(),
            Some("rl_recorded")
        );
        assert_eq!(super::command_id().as_deref(), Some("cmd_first"));
        assert_eq!(super::command_id().as_deref(), Some("cmd_second"));
        assert!(std::panic::catch_unwind(super::command_id).is_err());
    }

    #[test]
    fn recorded_decisions() -> Result<(), Box<dyn std::error::Error>> {
        let path =
            std::env::temp_dir().join(format!("replay-decisions-{}.json", std::process::id()));
        let run = || -> Result<bool, Box<dyn std::error::Error>> {
            Ok(std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", "replay::tests::child"])
                .env("REPLAY_DECISION_TEST", "1")
                .env("APPLE_BRIDGE_LAB_REPLAY", &path)
                .output()?
                .status
                .success())
        };
        std::fs::write(
            &path,
            r#"{"now":"2026-11-03T12:00:00.123456789Z","aliases":{"reminders\u0000generation\u0000reminders.list\u0000native":"rl_recorded"},"command_ids":["cmd_first","cmd_second"]}"#,
        )?;
        assert!(run()?);
        std::fs::write(
            &path,
            r#"{"now":"2026-11-03T12:00:00.123456789Z","aliases":{}}"#,
        )?;
        assert!(!run()?);
        std::fs::write(&path, r#"{"now":"broken"}"#)?;
        assert!(!run()?);
        std::fs::write(&path, "{")?;
        assert!(!run()?);
        std::fs::remove_file(path)?;
        assert!(!run()?);
        Ok(())
    }
}
