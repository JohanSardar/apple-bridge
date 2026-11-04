use ab_session::{
    Error,
    guard::{DispatchGuard, PreparedEffect},
};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn exclusive_guard_reserves_before_permit_and_survives_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::temp_dir().join(format!(
        "ab-guard-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut guard = DispatchGuard::open(&home)?;
        assert!(DispatchGuard::open(&home).is_err());
        let prepared = || PreparedEffect {
            command_id: "synthetic-command".into(),
            owner_fence: 1,
            ticket: "ticket".into(),
            request_hash: "hash".into(),
            operation: "focus.set".into(),
            expires_at: u64::MAX,
            prepared: json!({"request_hash":"hash"}),
        };
        let permit = guard.begin(prepared())?;
        assert_eq!(
            serde_json::to_value(permit)?["command_id"],
            "synthetic-command"
        );
        assert!(matches!(
            guard.begin(prepared()),
            Err(Error::AlreadyStarted)
        ));
        drop(guard);
        OpenOptions::new()
            .append(true)
            .open(home.join("dispatch-guard.log"))?
            .write_all(b"{partial")?;
        let mut recovered = DispatchGuard::open(&home)?;
        assert_eq!(
            recovered
                .outcome("synthetic-command")
                .ok_or("missing recovery")?["disposition"],
            "indeterminate"
        );
        assert!(matches!(
            recovered.begin(prepared()),
            Err(Error::AlreadyStarted)
        ));
        recovered.finish(
            "synthetic-command",
            "locally_observed",
            &json!({"observed":true}),
        )?;
        drop(recovered);
        let recovered = DispatchGuard::open(&home)?;
        assert_eq!(
            recovered
                .outcome("synthetic-command")
                .ok_or("missing outcome")?["disposition"],
            "locally_observed"
        );
        Ok(())
    })();
    std::fs::remove_dir_all(home)?;
    result
}
