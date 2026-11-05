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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_scan_and_urgent_work_bypass_poll_delay() {
        let now = Instant::now();
        let poll = Duration::from_secs(60);
        assert_eq!(Pace::default().due(now, poll, false, false), Ok(()));
        let pace = Pace {
            scanned: Some(now),
            poked: Some(now),
            cost: Duration::from_secs(1),
        };
        assert_eq!(pace.due(now, poll, false, true), Ok(()));
        assert_eq!(pace.due(now, poll, false, false), Err(now + poll));
        assert_eq!(pace.due(now + poll, poll, false, false), Ok(()));
    }

    #[test]
    fn touched_work_observes_scan_cost_and_never_delays_the_regular_poll() {
        let now = Instant::now();
        let poll = Duration::from_secs(60);
        let pace = Pace {
            scanned: Some(now),
            poked: Some(now),
            cost: Duration::from_secs(2),
        };
        assert_eq!(
            pace.due(now, poll, true, false),
            Err(now + Duration::from_secs(32))
        );
        let costly = Pace {
            cost: Duration::from_secs(10),
            ..pace
        };
        assert_eq!(costly.due(now, poll, true, false), Err(now + poll));
    }
}
