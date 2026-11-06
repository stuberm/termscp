//! Repaint only changed screens; cap background/timer work without delaying input.
use std::time::{Duration, Instant};

const BACKGROUND_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct Redraw {
    last_frame: Option<Instant>,
    dirty: bool,
    immediate: bool,
}

impl Redraw {
    pub fn request(&mut self, immediate: bool) {
        self.dirty = true;
        self.immediate |= immediate;
    }

    pub fn due(&mut self, now: Instant, timer_running: bool) -> bool {
        let due = self.last_frame.is_none()
            || self.immediate
            || ((self.dirty || timer_running)
                && self.last_frame.is_some_and(|last| {
                    now.saturating_duration_since(last) >= BACKGROUND_INTERVAL
                }));
        if due {
            self.last_frame = Some(now);
            self.dirty = false;
            self.immediate = false;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_screens_do_not_repaint_but_input_and_resize_are_immediate() {
        let now = Instant::now();
        let mut redraw = Redraw::default();
        assert!(redraw.due(now, false));
        assert!(!redraw.due(now + Duration::from_secs(1), false));
        redraw.request(true);
        assert!(redraw.due(now + Duration::from_secs(1), false));
        redraw.request(true);
        assert!(redraw.due(now + Duration::from_secs(1), false));
        assert!(!redraw.due(now + Duration::from_secs(2), false));
    }

    #[test]
    fn background_bursts_are_coalesced_without_losing_the_final_update() {
        let now = Instant::now();
        let mut redraw = Redraw::default();
        assert!(redraw.due(now, false));
        for milliseconds in [10, 20, 50, 90] {
            redraw.request(false);
            assert!(!redraw.due(now + Duration::from_millis(milliseconds), false));
        }
        assert!(redraw.due(now + BACKGROUND_INTERVAL, false));
        assert!(!redraw.due(now + Duration::from_secs(1), false));
    }

    #[test]
    fn running_timer_repaints_at_ten_hz_and_stops_when_finished() {
        let now = Instant::now();
        let mut redraw = Redraw::default();
        assert!(redraw.due(now, true));
        assert!(!redraw.due(now + Duration::from_millis(50), true));
        assert!(redraw.due(now + BACKGROUND_INTERVAL, true));
        redraw.request(false); // Finished event must still reach the screen.
        assert!(redraw.due(now + BACKGROUND_INTERVAL * 2, false));
        assert!(!redraw.due(now + Duration::from_secs(5), false));
    }
}
