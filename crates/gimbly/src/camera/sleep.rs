use std::time::{Duration, Instant};

/// The Tiny 3 Lite can return the previous sleep flag for more than two
/// seconds after accepting a command. Wait for confirmation, but let later
/// reads correct the displayed state if the command never takes effect.
const CONFIRM_WITHIN: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct SleepReadback {
    pending: Option<(bool, Instant)>,
}

impl SleepReadback {
    /// Call only after a successful write. A new request supersedes the old one.
    pub(super) fn sent(&mut self, sleeping: bool, now: Instant) {
        self.pending = Some((sleeping, now + CONFIRM_WITHIN));
    }

    pub(super) fn observe(&mut self, sleeping: bool, now: Instant) -> Option<bool> {
        if let Some((requested, deadline)) = self.pending
            && sleeping != requested
            && now < deadline
        {
            return None;
        }
        self.pending = None;
        Some(sleeping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_stale_awake_reads_until_sleep_is_confirmed() {
        let start = Instant::now();
        let mut readback = SleepReadback::default();
        readback.sent(true, start);
        for secs in [1, 2] {
            assert_eq!(
                readback.observe(false, start + Duration::from_secs(secs)),
                None
            );
        }
        assert_eq!(
            readback.observe(true, start + Duration::from_secs(3)),
            Some(true)
        );
        // A real external wake is still reflected once Sleep was confirmed.
        assert_eq!(
            readback.observe(false, start + Duration::from_secs(4)),
            Some(false)
        );
    }

    #[test]
    fn resume_supersedes_a_sleep_that_is_still_pending() {
        let start = Instant::now();
        let mut readback = SleepReadback::default();
        readback.sent(true, start);
        let resume = start + Duration::from_secs(1);
        readback.sent(false, resume);
        assert_eq!(
            readback.observe(true, resume + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            readback.observe(false, resume + Duration::from_secs(2)),
            Some(false)
        );
    }

    #[test]
    fn reads_can_correct_an_unconfirmed_command_after_the_deadline() {
        let start = Instant::now();
        let mut readback = SleepReadback::default();
        readback.sent(true, start);
        assert_eq!(readback.observe(false, start + CONFIRM_WITHIN), Some(false));
    }
}
