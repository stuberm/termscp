//! Monotonic wall-clock timing for a folder comparison, independent of messages
//! and text-diff/copy activity. Finished totals come from the worker, not UI polls.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Completed,
    Incomplete,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Debug, Default)]
pub(super) enum ComparisonTimer {
    #[default]
    Idle,
    Running(Instant),
    Finished {
        elapsed: Duration,
        outcome: Outcome,
    },
}

pub(super) fn duration_label(elapsed: Duration) -> String {
    format!("{:.3}s", elapsed.as_secs_f64())
}

impl ComparisonTimer {
    pub fn start(&mut self, started: Instant) {
        *self = Self::Running(started);
    }

    pub fn finish(&mut self, elapsed: Duration, outcome: Outcome) {
        *self = Self::Finished { elapsed, outcome };
    }

    /// Used only when abandoning a running scan or losing its worker. Completed
    /// scan totals must survive subsequent text diffs, copies and navigation.
    pub fn stop(&mut self, outcome: Outcome) {
        if let Self::Running(started) = self {
            let elapsed = started.elapsed();
            self.finish(elapsed, outcome);
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Idle => "Comparison: not started".into(),
            Self::Running(started) => format!(
                "Comparison: running · {} elapsed",
                duration_label(started.elapsed())
            ),
            Self::Finished { elapsed, outcome } => format!(
                "Comparison: {} {}",
                match outcome {
                    Outcome::Completed => "completed in",
                    Outcome::Incomplete => "incomplete after",
                    Outcome::Cancelled => "cancelled after",
                    Outcome::Failed => "failed after",
                    Outcome::Interrupted => "interrupted after",
                },
                duration_label(*elapsed),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_use_seconds_with_millisecond_precision() {
        for (duration, expected) in [
            (Duration::ZERO, "0.000s"),
            (Duration::from_millis(1), "0.001s"),
            (Duration::from_millis(12345), "12.345s"),
            (Duration::from_millis(61234), "61.234s"),
            (Duration::from_secs(3600), "3600.000s"),
        ] {
            assert_eq!(duration_label(duration), expected);
        }
    }

    #[test]
    fn live_timer_freezes_worker_total_and_resets_for_the_next_scan() {
        let mut timer = ComparisonTimer::default();
        assert_eq!(timer.label(), "Comparison: not started");
        timer.start(Instant::now() - Duration::from_secs(10));
        assert!(timer.label().starts_with("Comparison: running · "));
        assert!(
            matches!(timer, ComparisonTimer::Running(started) if started.elapsed() >= Duration::from_secs(10))
        );
        // A delayed UI poll must not inflate the worker's recorded duration.
        timer.finish(Duration::from_millis(1234), Outcome::Completed);
        assert_eq!(timer.label(), "Comparison: completed in 1.234s");
        timer.stop(Outcome::Interrupted);
        assert_eq!(timer.label(), "Comparison: completed in 1.234s");
        timer.start(Instant::now());
        assert!(timer.label().starts_with("Comparison: running · "));
    }

    #[test]
    fn partial_or_failed_scans_are_never_labeled_completed() {
        let mut timer = ComparisonTimer::default();
        for outcome in [
            Outcome::Incomplete,
            Outcome::Cancelled,
            Outcome::Failed,
            Outcome::Interrupted,
        ] {
            timer.finish(Duration::from_millis(1250), outcome);
            assert!(!timer.label().contains("completed"));
            assert!(timer.label().ends_with("after 1.250s"));
        }
        timer.start(Instant::now());
        timer.stop(Outcome::Interrupted);
        assert!(timer.label().starts_with("Comparison: interrupted after "));
    }
}
