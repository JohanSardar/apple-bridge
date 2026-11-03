use std::fmt::Debug;
pub trait Clock: Debug + Send + Sync {
    fn now(&self) -> jiff::Timestamp;
}
#[derive(Debug)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> jiff::Timestamp {
        jiff::Timestamp::now()
    }
}
#[cfg(feature = "lab-harness")]
#[derive(Debug)]
pub struct LabClock(pub std::path::PathBuf);
#[cfg(feature = "lab-harness")]
impl Clock for LabClock {
    fn now(&self) -> jiff::Timestamp {
        if std::env::var_os("APPLE_BRIDGE_LAB_REPLAY").is_some() {
            return ab_state::replay::now();
        }
        let current = jiff::Timestamp::now();
        let offset = std::fs::read_to_string(self.0.join("clock-offset"))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        current
            .checked_add(jiff::SignedDuration::from_millis(offset))
            .unwrap_or(current)
    }
}
