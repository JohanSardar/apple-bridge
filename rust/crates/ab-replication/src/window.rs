use crate::{Cursor, Error};
use ab_trust::wire::messages::FeedFrame;
use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct Window {
    available: usize,
    pending: VecDeque<(usize, Option<Cursor>)>,
    deadline: Instant,
}
impl Window {
    pub fn new(limit: usize) -> Self {
        Self {
            available: limit,
            pending: VecDeque::new(),
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    pub fn reserve(&mut self, size: usize, cursor: Option<Cursor>) -> bool {
        if size > self.available {
            return false;
        }
        if self.pending.is_empty() {
            self.deadline = Instant::now() + Duration::from_secs(30);
        }
        self.available -= size;
        self.pending.push_back((size, cursor));
        true
    }
    pub fn acknowledge(&mut self, ack: &FeedFrame) -> Result<(), Error> {
        let (size, expected) = self.pending.front().ok_or(Error::Invalid)?;
        if usize::try_from(ack.credits).ok() != Some(*size) {
            return Err(Error::Invalid);
        }
        if let Some(expected) = expected {
            let actual: Cursor =
                serde_json::from_str(ack.cursor.as_ref().ok_or(Error::Invalid)?.get())?;
            if actual != *expected {
                return Err(Error::Invalid);
            }
        }
        self.available += size;
        self.pending.pop_front();
        self.deadline = Instant::now() + Duration::from_secs(30);
        Ok(())
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}
