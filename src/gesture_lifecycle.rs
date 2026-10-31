use std::time::{Duration, Instant};

pub const WATCHDOG_INTERVAL_MS: u32 = 100;
const RELEASE_GRACE: Duration = Duration::from_millis(150);
const MAX_SESSION_DURATION: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    MissingRelease,
    Timeout,
    DeferredCleanup,
    NewPress,
    InconsistentState,
}

impl RecoveryReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingRelease => "触发键已松开但未收到结束事件",
            Self::Timeout => "手势持续超过 15 秒",
            Self::DeferredCleanup => "结束处理被中断，已清理遗留状态",
            Self::NewPress => "新按下事件到达，已取消上一笔遗留轨迹",
            Self::InconsistentState => "手势标志与轨迹状态不一致",
        }
    }
}

pub struct SessionWatchdog {
    started_at: Instant,
    released_since: Option<Instant>,
}

impl SessionWatchdog {
    pub fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            released_since: None,
        }
    }

    // Raw release is an independent completion signal. Allow the normal hook
    // handler to finish before cancelling a session whose release was lost.
    pub fn poll(&mut self, now: Instant, release_observed: bool) -> Option<RecoveryReason> {
        if now.saturating_duration_since(self.started_at) >= MAX_SESSION_DURATION {
            return Some(RecoveryReason::Timeout);
        }
        if !release_observed {
            self.released_since = None;
        } else {
            let released_since = self.released_since.get_or_insert(now);
            if now.saturating_duration_since(*released_since) >= RELEASE_GRACE {
                return Some(RecoveryReason::MissingRelease);
            }
        }
        None
    }
}

pub fn accepts_overlay_event(session_id: usize, displayed_id: usize, active_id: usize) -> bool {
    session_id != 0 && session_id == displayed_id && session_id == active_id
}

#[cfg(test)]
mod tests {
    use super::{RecoveryReason, SessionWatchdog, accepts_overlay_event};
    use std::time::{Duration, Instant};

    #[test]
    fn raw_release_waits_for_the_normal_handler_before_recovering() {
        let start = Instant::now();
        let mut watchdog = SessionWatchdog::new(start);
        assert_eq!(watchdog.poll(start, false), None);
        assert_eq!(
            watchdog.poll(start + Duration::from_millis(100), true),
            None
        );
        assert_eq!(
            watchdog.poll(start + Duration::from_millis(200), false),
            None
        );
        assert_eq!(
            watchdog.poll(start + Duration::from_millis(300), true),
            None
        );
        assert_eq!(
            watchdog.poll(start + Duration::from_millis(500), true),
            Some(RecoveryReason::MissingRelease)
        );
    }

    #[test]
    fn a_stuck_pressed_state_also_has_a_bounded_lifetime() {
        let start = Instant::now();
        let mut watchdog = SessionWatchdog::new(start);
        assert_eq!(watchdog.poll(start + Duration::from_secs(14), false), None);
        assert_eq!(
            watchdog.poll(start + Duration::from_secs(15), false),
            Some(RecoveryReason::Timeout)
        );
    }

    #[test]
    fn old_overlay_messages_cannot_extend_a_finished_or_replaced_stroke() {
        assert!(accepts_overlay_event(1, 1, 1));
        assert!(!accepts_overlay_event(1, 1, 0));
        assert!(!accepts_overlay_event(1, 2, 2));
        assert!(!accepts_overlay_event(2, 1, 2));
        assert!(accepts_overlay_event(2, 2, 2));
    }
}
