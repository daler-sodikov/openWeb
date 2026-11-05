//! When to send the next frame: at most one per interval, at most one in
//! flight (the view acknowledges each), and never a queue. A frame that is
//! not acknowledged in time is treated as lost so a closed or hidden view
//! cannot stop a terminal for good.

use std::time::{Duration, Instant};

/// Frame pacing state of one terminal.
#[derive(Debug)]
pub struct Pacer {
    interval: Duration,
    ack_timeout: Duration,
    dirty: bool,
    last_sent: Option<Instant>,
    in_flight: Option<(u64, Instant)>,
    next_seq: u64,
}

/// What the frame thread should do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Build and send frame `seq`.
    Send(u64),
    /// Nothing to do until something changes.
    Idle,
    /// Check again after this long.
    Wait(Duration),
}

impl Pacer {
    /// A pacer sending at most once per `interval`, waiting at most
    /// `ack_timeout` for an acknowledgement.
    pub fn new(interval: Duration, ack_timeout: Duration) -> Self {
        Self {
            interval,
            ack_timeout,
            dirty: false,
            last_sent: None,
            in_flight: None,
            next_seq: 1,
        }
    }

    /// The screen changed.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The view acknowledged frame `seq`.
    pub fn ack(&mut self, seq: u64) {
        if self.in_flight.is_some_and(|(s, _)| s == seq) {
            self.in_flight = None;
        }
    }

    /// Decides at `now`; on `Send` the frame counts as sent and in flight.
    pub fn decide(&mut self, now: Instant) -> Decision {
        if let Some((_, sent)) = self.in_flight {
            let waited = now.saturating_duration_since(sent);
            if waited < self.ack_timeout {
                return Decision::Wait(self.ack_timeout - waited);
            }
            // Lost frame: stop waiting for it.
            self.in_flight = None;
        }
        if !self.dirty {
            return Decision::Idle;
        }
        if let Some(last) = self.last_sent {
            let since = now.saturating_duration_since(last);
            if since < self.interval {
                return Decision::Wait(self.interval - since);
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.dirty = false;
        self.last_sent = Some(now);
        self.in_flight = Some((seq, now));
        Decision::Send(seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn pacer() -> Pacer {
        Pacer::new(16 * MS, 1000 * MS)
    }

    #[test]
    fn coalesces_changes_into_one_frame_per_interval() {
        let t0 = Instant::now();
        let mut p = pacer();
        assert_eq!(p.decide(t0), Decision::Idle);
        for _ in 0..1000 {
            p.mark_dirty();
        }
        assert_eq!(p.decide(t0), Decision::Send(1));
        p.ack(1);
        p.mark_dirty();
        // Too soon after the last frame.
        assert_eq!(p.decide(t0 + 5 * MS), Decision::Wait(11 * MS));
        assert_eq!(p.decide(t0 + 16 * MS), Decision::Send(2));
    }

    #[test]
    fn at_most_one_frame_in_flight() {
        let t0 = Instant::now();
        let mut p = pacer();
        p.mark_dirty();
        assert_eq!(p.decide(t0), Decision::Send(1));
        p.mark_dirty();
        // Not acknowledged: wait even though the interval passed.
        assert_eq!(p.decide(t0 + 100 * MS), Decision::Wait(900 * MS));
        // A stale acknowledgement does not release it.
        p.ack(7);
        assert!(matches!(p.decide(t0 + 200 * MS), Decision::Wait(_)));
        p.ack(1);
        assert_eq!(p.decide(t0 + 200 * MS), Decision::Send(2));
    }

    #[test]
    fn a_lost_frame_does_not_stop_the_terminal() {
        let t0 = Instant::now();
        let mut p = pacer();
        p.mark_dirty();
        assert_eq!(p.decide(t0), Decision::Send(1));
        p.mark_dirty();
        assert_eq!(p.decide(t0 + 1000 * MS), Decision::Send(2));
    }
}
