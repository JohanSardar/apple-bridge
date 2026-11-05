use super::{SCAN_DUTY, SCAN_FLOOR};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(super) struct Pace {
    pub(super) scanned: Option<Instant>,
    pub(super) poked: Option<Instant>,
    pub(super) cost: Duration,
}

impl Pace {
    pub(super) fn due(
        &self,
        now: Instant,
        poll: Duration,
        touched: bool,
        urgent: bool,
    ) -> Result<(), Instant> {
        let Some(scanned) = self.scanned else {
            return Ok(());
        };
        if urgent {
            return Ok(());
        }
        let mut when = scanned + poll;
        if touched {
            let gap = poll.min(SCAN_FLOOR).max(self.cost * SCAN_DUTY);
            when = when.min(self.poked.map_or(now, |poked| poked + gap));
        }
        if now >= when { Ok(()) } else { Err(when) }
    }
}
