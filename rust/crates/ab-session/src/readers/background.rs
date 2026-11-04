use super::Error;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct Background<T> {
    slot: Arc<Mutex<Option<Result<T, Error>>>>,
    running: Arc<Mutex<bool>>,
    interval: Duration,
    last: Option<Instant>,
}

impl<T: Send + 'static> Background<T> {
    pub(super) fn new(interval: Duration) -> Self {
        Self {
            slot: Arc::default(),
            running: Arc::default(),
            interval,
            last: None,
        }
    }

    fn busy(&self) -> bool {
        self.running.lock().is_ok_and(|running| *running)
    }

    pub(super) fn start_if_due(
        &mut self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) {
        if self.busy() || self.last.is_some_and(|at| at.elapsed() < self.interval) {
            return;
        }
        self.last = Some(Instant::now());
        let (slot, running) = (Arc::clone(&self.slot), Arc::clone(&self.running));
        if let Ok(mut flag) = running.lock() {
            *flag = true;
        }
        thread::spawn(move || {
            let answer = work();
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(answer);
            }
            if let Ok(mut flag) = running.lock() {
                *flag = false;
            }
        });
    }

    pub(super) fn take(&self) -> Option<Result<T, Error>> {
        self.slot.lock().ok()?.take()
    }

    pub(super) fn wait(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        while self.busy() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
    }
}
