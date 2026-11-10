use std::{
    collections::BTreeSet,
    process::Child,
    sync::Mutex,
    time::{Duration, Instant},
};

static OWNED: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

#[derive(Debug)]
pub struct Owned(pub Child);

impl Owned {
    pub fn new(child: Child) -> Self {
        if let Ok(mut owned) = OWNED.lock() {
            owned.insert(child.id());
        }
        Self(child)
    }

    pub fn stop(&mut self) {
        ab_sys::terminate(self.0.id());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Ok(mut owned) = OWNED.lock() {
            owned.remove(&self.0.id());
        }
    }
}

pub fn stop_all() {
    let pids: Vec<u32> = OWNED
        .lock()
        .map(|owned| owned.iter().copied().collect())
        .unwrap_or_default();
    for pid in &pids {
        ab_sys::terminate(*pid);
    }
    if !pids.is_empty() {
        std::thread::sleep(Duration::from_millis(300));
    }
}
